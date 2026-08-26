//! A main-thread message loop driven by a Core Foundation run loop source.

#![cfg(target_os = "macos")]
#![forbid(unsafe_code)]

pub use messageloop_core::{SendError, Sender, SenderResult};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};
use zpd_appkit::actor::{
    ActorError, ActorId, ActorRef, ApplicationMessage, EventRouteToken, WindowEvent,
    WindowEventKind,
};
use zpd_appkit::runloop::{
    Application, ApplicationDelegate, ApplicationError, ContextRunLoopSource, RunLoopSourceError,
    RunLoopSourceSignaler, WindowEventRegistration,
};
use zpd_appkit::ui::{Window, WindowError};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLoopError {
    NotMainThread,
    NativeCreationFailed,
    Application(ActorError),
    EventHandler(ApplicationError),
}

impl std::fmt::Display for MessageLoopError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMainThread => {
                formatter.write_str("the AppKit message loop requires the main thread")
            }
            Self::NativeCreationFailed => {
                formatter.write_str("Core Foundation failed to create a run-loop source")
            }
            Self::Application(error) => error.fmt(formatter),
            Self::EventHandler(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for MessageLoopError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Application(error) => Some(error),
            Self::EventHandler(error) => Some(error),
            _ => None,
        }
    }
}

impl From<RunLoopSourceError> for MessageLoopError {
    fn from(error: RunLoopSourceError) -> Self {
        match error {
            RunLoopSourceError::NotCurrent => Self::NotMainThread,
            RunLoopSourceError::NativeCreationFailed => Self::NativeCreationFailed,
        }
    }
}

enum Queued<M> {
    User(M),
    Window(WindowEvent),
}

struct QueueState<M> {
    messages: VecDeque<Queued<M>>,
    quit_requested: bool,
    closed: bool,
    error: Option<MessageLoopError>,
    source: Option<RunLoopSourceSignaler>,
}

struct SharedState<M> {
    queue: Mutex<QueueState<M>>,
}

impl<M> SharedState<M> {
    fn new() -> Self {
        Self {
            queue: Mutex::new(QueueState {
                messages: VecDeque::new(),
                quit_requested: false,
                closed: false,
                error: None,
                source: None,
            }),
        }
    }

    fn signal_locked(state: &QueueState<M>) {
        let Some(source) = &state.source else {
            return;
        };
        let signaled = source.signal();
        debug_assert!(signaled, "a registered run-loop source must be active");
    }

    fn request_termination(&self) {
        let mut state = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed {
            return;
        }
        state.quit_requested = true;
        Self::signal_locked(&state);
    }
}

/// Thread-safe sender for [`MessageLoopAppkit`].
pub struct AppkitSender<M> {
    state: Weak<SharedState<M>>,
}

impl<M> Clone for AppkitSender<M> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<M: Send + 'static> Sender for AppkitSender<M> {
    type Message = M;

    fn send(&self, message: M) -> SenderResult {
        let shared = self.state.upgrade().ok_or(SendError::Closed)?;
        let mut state = shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.closed || state.quit_requested || state.source.is_none() {
            return Err(SendError::Closed);
        }
        state.messages.push_back(Queued::User(message));
        SharedState::signal_locked(&state);
        Ok(())
    }
}

/// Context available only while a handler callback is running.
///
/// It provides access to the message loop's Window registry for the borrowed
/// Application session. The marker intentionally makes it neither `Send` nor
/// `Sync`.
pub struct Context<'callback, 'application, M> {
    shared: &'callback Arc<SharedState<M>>,
    windows: &'callback WindowRegistry<'application>,
    local_only: PhantomData<Rc<()>>,
}

impl<'application, M: Send + 'static> Context<'_, 'application, M> {
    #[must_use]
    pub fn sender(&self) -> AppkitSender<M> {
        AppkitSender {
            state: Arc::downgrade(self.shared),
        }
    }

    /// Requests termination after the current callback returns.
    pub fn request_termination(&self) {
        self.shared.request_termination();
    }

    /// Creates a Window owned by this message loop and returns its Actor ID.
    pub fn create_window(&self) -> Result<ActorId, WindowError> {
        self.create_window_with_event_route(None)
    }

    /// Creates an owned Window whose Actor carries `event_route` before its
    /// `Created` event is emitted.
    pub fn create_window_with_event_route(
        &self,
        event_route: Option<EventRouteToken>,
    ) -> Result<ActorId, WindowError> {
        self.windows.create(event_route)
    }

    #[must_use]
    /// Returns whether the message loop still owns the identified Window.
    pub fn contains_window(&self, id: ActorId) -> bool {
        self.windows.windows.borrow().contains_key(&id)
    }

    /// Runs an operation against an owned Window without transferring ownership.
    ///
    /// The operation must not mutate the Window registry through this Context.
    pub fn with_window<R>(
        &self,
        id: ActorId,
        operation: impl FnOnce(&Window<'application>) -> R,
    ) -> Option<R> {
        self.windows.windows.borrow().get(&id).map(operation)
    }

    /// Closes and removes an owned Window.
    pub fn remove_window(&self, id: ActorId) -> bool {
        self.windows.windows.borrow_mut().remove(&id).is_some()
    }
}

/// Receives messages serially from the main run loop source.
pub trait MessageLoopHandler<M> {
    fn init(&mut self, _cx: &Context<'_, '_, M>) {}

    fn on(&mut self, _cx: &Context<'_, '_, M>, _message: M) {}

    fn terminate(&mut self, _cx: &Context<'_, '_, M>) {}
}

/// Compatibility name matching `messageloop-sync`.
pub use MessageLoopHandler as MessageHandler;

struct WindowRegistry<'application> {
    create_window: Box<
        dyn Fn(Option<EventRouteToken>) -> Result<Window<'application>, WindowError> + 'application,
    >,
    windows: RefCell<HashMap<ActorId, Window<'application>>>,
}

impl WindowRegistry<'_> {
    fn create(&self, event_route: Option<EventRouteToken>) -> Result<ActorId, WindowError> {
        let window = (self.create_window)(event_route)?;
        let id = window.actor_ref().actor_id();
        self.windows.borrow_mut().insert(id, window);
        Ok(id)
    }
}

struct CallbackState<'application, M, H> {
    shared: Arc<SharedState<M>>,
    windows: WindowRegistry<'application>,
    handler: RefCell<H>,
    initialized: Cell<bool>,
    terminated: Cell<bool>,
    performing: Cell<bool>,
}

impl<M, H> CallbackState<'_, M, H>
where
    M: From<WindowEvent> + Send + 'static,
    H: MessageLoopHandler<M>,
{
    fn perform(&self) -> bool {
        // A nested CFRunLoop invocation must not borrow the handler twice. The
        // outer invocation will drain anything queued by the nested loop.
        if self.performing.replace(true) {
            return false;
        }

        let cx = Context {
            shared: &self.shared,
            windows: &self.windows,
            local_only: PhantomData,
        };
        if !self.initialized.replace(true) {
            self.handler.borrow_mut().init(&cx);
        }

        loop {
            let message = {
                let mut state = self
                    .shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.quit_requested {
                    None
                } else {
                    state.messages.pop_front()
                }
            };
            let Some(message) = message else {
                break;
            };
            let message = match message {
                Queued::User(message) => message,
                Queued::Window(event) => {
                    if matches!(event.kind, WindowEventKind::DidClose) {
                        self.windows.windows.borrow_mut().remove(&event.window);
                    }
                    M::from(event)
                }
            };
            self.handler.borrow_mut().on(&cx, message);
        }

        let quit_requested = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .quit_requested;
        let should_stop = quit_requested && !self.terminated.replace(true);
        if should_stop {
            {
                let mut state = self
                    .shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.closed = true;
                state.messages.clear();
            }
            self.handler.borrow_mut().terminate(&cx);
        }

        self.performing.set(false);
        should_stop
    }
}

/// A single-source dispatcher owned and run by the process main thread.
///
/// This message loop borrows the supplied [`Application`] session, owns every
/// Window created through [`Context`], and installs one semantic Window event
/// callback. Native callbacks are queued and converted through
/// `M: From<WindowEvent>` before [`MessageLoopHandler::on`] runs.
pub struct MessageLoopAppkit<'application, M, H> {
    application: ActorRef,
    shared: Arc<SharedState<M>>,
    callback: Rc<CallbackState<'application, M, H>>,
    event_registration: Option<WindowEventRegistration<'application>>,
    _source: ContextRunLoopSource<SourceContext<'application, M, H>>,
}

struct SourceContext<'application, M, H> {
    callback: Rc<CallbackState<'application, M, H>>,
    application: ActorRef,
}

fn perform_source<M, H>(source: &SourceContext<'_, M, H>)
where
    M: From<WindowEvent> + Send + 'static,
    H: MessageLoopHandler<M>,
{
    if source.callback.perform()
        && let Err(error) = source.application.send(ApplicationMessage::Stop)
    {
        let mut state = source
            .callback
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state
            .error
            .get_or_insert(MessageLoopError::Application(error));
    }
}

impl<'application, M, H> MessageLoopAppkit<'application, M, H>
where
    M: From<WindowEvent> + Send + 'static,
    H: MessageLoopHandler<M>,
{
    /// Creates one source on the supplied application's main run loop.
    ///
    /// # Errors
    /// Returns an error when called off that run loop, native source creation
    /// fails, or the Application already has a Window event callback.
    pub fn new<D: ApplicationDelegate>(
        application: &'application Application<D>,
        handler: H,
    ) -> Result<Self, MessageLoopError> {
        let application_actor = application.actor_ref();
        if !application_actor.is_alive() {
            return Err(MessageLoopError::Application(ActorError::NotActive));
        }
        let shared = Arc::new(SharedState::new());
        let callback = Rc::new(CallbackState {
            shared: shared.clone(),
            windows: WindowRegistry {
                create_window: Box::new(move |route| {
                    application.create_window_with_event_route(route)
                }),
                windows: RefCell::new(HashMap::new()),
            },
            handler: RefCell::new(handler),
            initialized: Cell::new(false),
            terminated: Cell::new(false),
            performing: Cell::new(false),
        });
        let weak = Arc::downgrade(&shared);
        let event_registration = application
            .on(move |event| {
                let Some(shared) = weak.upgrade() else { return };
                let mut state = shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if state.closed || state.quit_requested {
                    return;
                }
                state.messages.push_back(Queued::Window(event));
                SharedState::signal_locked(&state);
            })
            .map_err(MessageLoopError::EventHandler)?;
        let source = application.run_loop().create_context_source(
            SourceContext {
                callback: callback.clone(),
                application: application_actor.clone(),
            },
            perform_source::<M, H>,
        )?;
        {
            let mut state = shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.source = Some(source.signaler());
        }

        Ok(Self {
            application: application_actor,
            shared,
            callback,
            event_registration: Some(event_registration),
            _source: source,
        })
    }

    #[must_use]
    pub fn sender(&self) -> AppkitSender<M> {
        AppkitSender {
            state: Arc::downgrade(&self.shared),
        }
    }

    /// Runs the application's `AppKit` event loop until termination.
    ///
    /// # Errors
    /// Returns an error if the originating Application session has expired or stopping it fails.
    pub fn run(self) -> Result<(), MessageLoopError> {
        {
            let state = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            SharedState::signal_locked(&state);
        }
        self.application
            .send(ApplicationMessage::Run)
            .map_err(MessageLoopError::Application)?;
        let mut state = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.error.take().map_or(Ok(()), Err)
    }
}

impl<M, H> Drop for MessageLoopAppkit<'_, M, H> {
    fn drop(&mut self) {
        self.event_registration.take();
        {
            let mut state = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.closed = true;
            state.messages.clear();
            state.source = None;
        }
        self.callback.windows.windows.borrow_mut().clear();
    }
}

/// Compatibility name matching the existing native implementation.
pub type AppkitMessageLoop<'application, M, H> = MessageLoopAppkit<'application, M, H>;
