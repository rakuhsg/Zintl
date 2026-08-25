//! A main-thread message loop driven by a Core Foundation run loop source.

#![cfg(target_os = "macos")]
#![forbid(unsafe_code)]

pub use messageloop_core::{SendError, Sender, SenderResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};
use zpd_appkit::actor::{ActorError, ActorRef, ApplicationMessage};
use zpd_appkit::runloop::{
    Application, ApplicationDelegate, ContextRunLoopSource, RunLoopSourceError,
    RunLoopSourceSignaler,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLoopError {
    NotMainThread,
    NativeCreationFailed,
    Application(ActorError),
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
        }
    }
}

impl std::error::Error for MessageLoopError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Application(error) => Some(error),
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

struct QueueState<M> {
    messages: VecDeque<M>,
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
        state.messages.push_back(message);
        SharedState::signal_locked(&state);
        Ok(())
    }
}

/// Context available only while a handler callback is running.
///
/// The marker intentionally makes this type neither `Send` nor `Sync`.
pub struct Context<'a, M> {
    shared: &'a Arc<SharedState<M>>,
    local_only: PhantomData<Rc<()>>,
}

impl<M: Send + 'static> Context<'_, M> {
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
}

/// Receives messages serially from the main run loop source.
pub trait MessageLoopHandler<M> {
    fn init(&mut self, _cx: &Context<'_, M>) {}

    fn on(&mut self, _cx: &Context<'_, M>, _message: M) {}

    fn terminate(&mut self, _cx: &Context<'_, M>) {}
}

/// Compatibility name matching `messageloop-sync`.
pub use MessageLoopHandler as MessageHandler;

struct CallbackState<M, H> {
    shared: Arc<SharedState<M>>,
    handler: RefCell<H>,
    initialized: Cell<bool>,
    terminated: Cell<bool>,
    performing: Cell<bool>,
}

impl<M: Send + 'static, H: MessageLoopHandler<M>> CallbackState<M, H> {
    fn perform(&self) -> bool {
        // A nested CFRunLoop invocation must not borrow the handler twice. The
        // outer invocation will drain anything queued by the nested loop.
        if self.performing.replace(true) {
            return false;
        }

        let cx = Context {
            shared: &self.shared,
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
/// This message loop holds a non-owning actor handle to the supplied
/// [`Application`] session. It installs its source on that application's native
/// run loop and uses the handle to drive and stop the `AppKit` event loop.
/// Running the loop after that session ends returns [`ActorError::NotActive`].
pub struct MessageLoopAppkit<M, H> {
    application: ActorRef,
    shared: Arc<SharedState<M>>,
    _source: ContextRunLoopSource<SourceContext<M, H>>,
}

struct SourceContext<M, H> {
    callback: Rc<CallbackState<M, H>>,
    application: ActorRef,
}

fn perform_source<M, H>(source: &SourceContext<M, H>)
where
    M: Send + 'static,
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

impl<M, H> MessageLoopAppkit<M, H>
where
    M: Send + 'static,
    H: MessageLoopHandler<M>,
{
    /// Creates one source on the supplied application's main run loop.
    ///
    /// # Errors
    /// Returns an error when called off that run loop or native source
    /// creation fails.
    pub fn new<D: ApplicationDelegate>(
        application: &Application<D>,
        handler: H,
    ) -> Result<Self, MessageLoopError> {
        let application_actor = application.actor_ref();
        if !application_actor.is_alive() {
            return Err(MessageLoopError::Application(ActorError::NotActive));
        }
        let shared = Arc::new(SharedState::new());
        let callback = Rc::new(CallbackState {
            shared: shared.clone(),
            handler: RefCell::new(handler),
            initialized: Cell::new(false),
            terminated: Cell::new(false),
            performing: Cell::new(false),
        });
        let source = application.run_loop().create_context_source(
            SourceContext {
                callback,
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

impl<M, H> Drop for MessageLoopAppkit<M, H> {
    fn drop(&mut self) {
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
    }
}

/// Compatibility name matching the existing native implementation.
pub type AppkitMessageLoop<M, H> = MessageLoopAppkit<M, H>;
