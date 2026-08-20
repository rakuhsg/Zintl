//! A main-thread message loop driven by a `WinUI` `DispatcherQueue`.

#![cfg(target_os = "windows")]
#![forbid(unsafe_code)]

pub use messageloop_core::{SendError, Sender, SenderResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::error::Error;
use std::fmt;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};
use zpd_winui3::{Application, ApplicationContext, DispatcherQueueSignaler, DispatcherQueueSource};

struct QueueState<M> {
    messages: VecDeque<M>,
    quit_requested: bool,
    closed: bool,
    signaler: Option<DispatcherQueueSignaler>,
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
                signaler: None,
            }),
        }
    }

    fn signal(&self) -> bool {
        let signaler = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .signaler
            .clone();
        signaler.is_none_or(|signaler| signaler.signal())
    }

    fn request_termination(&self) {
        {
            let mut state = self
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed {
                return;
            }
            state.quit_requested = true;
        }
        let _ = self.signal();
    }

    fn close(&self) {
        let mut state = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.closed = true;
        state.messages.clear();
        state.signaler = None;
    }
}

/// Thread-safe sender for [`MessageLoopDispatcherQueue`].
pub struct DispatcherQueueSender<M> {
    state: Weak<SharedState<M>>,
}

impl<M> Clone for DispatcherQueueSender<M> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<M: Send + 'static> Sender for DispatcherQueueSender<M> {
    type Message = M;

    fn send(&self, message: M) -> SenderResult {
        let shared = self.state.upgrade().ok_or(SendError::Closed)?;
        let signaler = {
            let mut state = shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.closed || state.quit_requested {
                return Err(SendError::Closed);
            }
            state.messages.push_back(message);
            state.signaler.clone()
        };
        let Some(signaler) = signaler else {
            // Messages sent before OnLaunched remain queued for the first signal.
            return Ok(());
        };
        if signaler.signal() {
            Ok(())
        } else {
            shared.close();
            Err(SendError::Closed)
        }
    }
}

/// UI-thread context available while a handler callback is running.
pub struct Context<'a, M> {
    application: &'a ApplicationContext,
    shared: &'a Arc<SharedState<M>>,
    local_only: PhantomData<Rc<()>>,
}

impl<M: Send + 'static> Context<'_, M> {
    #[must_use]
    pub fn sender(&self) -> DispatcherQueueSender<M> {
        DispatcherQueueSender {
            state: Arc::downgrade(self.shared),
        }
    }

    pub fn request_termination(&self) {
        self.shared.request_termination();
    }

    #[must_use]
    pub fn application(&self) -> &ApplicationContext {
        self.application
    }
}

/// Receives messages serially on the `WinUI` application thread.
pub trait MessageLoopHandler<M> {
    fn init(&mut self, _cx: &Context<'_, M>) {}

    fn on(&mut self, _cx: &Context<'_, M>, _message: M) {}

    fn terminate(&mut self, _cx: &Context<'_, M>) {}
}

pub use MessageLoopHandler as MessageHandler;

struct CallbackState<M, H> {
    application: ApplicationContext,
    shared: Arc<SharedState<M>>,
    handler: RefCell<H>,
    initialized: Cell<bool>,
    terminated: Cell<bool>,
    performing: Cell<bool>,
}

impl<M: Send + 'static, H: MessageLoopHandler<M>> CallbackState<M, H> {
    fn context(&self) -> Context<'_, M> {
        Context {
            application: &self.application,
            shared: &self.shared,
            local_only: PhantomData,
        }
    }

    fn initialize(&self) {
        if !self.initialized.replace(true) {
            self.handler.borrow_mut().init(&self.context());
        }
    }

    fn finish(&self) {
        if self.terminated.replace(true) {
            return;
        }
        self.initialize();
        self.shared.close();
        self.handler.borrow_mut().terminate(&self.context());
    }

    fn perform(&self) {
        // Nested message pumps must not mutably borrow the handler twice.
        if self.performing.replace(true) {
            return;
        }
        self.initialize();

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
            self.handler.borrow_mut().on(&self.context(), message);
        }

        let should_exit = self
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .quit_requested;
        if should_exit {
            self.finish();
        }
        self.performing.set(false);
        if should_exit {
            let _ = self.application.exit();
        }
    }
}

struct RuntimeState<M: Send + 'static, H: MessageLoopHandler<M> + 'static> {
    source: Option<DispatcherQueueSource<'static>>,
    callback: Rc<CallbackState<M, H>>,
}

impl<M: Send + 'static, H: MessageLoopHandler<M> + 'static> Drop for RuntimeState<M, H> {
    fn drop(&mut self) {
        self.callback.finish();
        // Drop the native callback before releasing its final Rust state.
        self.source.take();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MessageLoopError {
    Application(zpd_winui3::Error),
    Source(zpd_winui3::Error),
    DispatcherClosed,
}

impl fmt::Display for MessageLoopError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Application(error) => write!(formatter, "failed to run WinUI: {error}"),
            Self::Source(error) => write!(formatter, "failed to create dispatcher source: {error}"),
            Self::DispatcherClosed => formatter.write_str("DispatcherQueue closed during setup"),
        }
    }
}

impl Error for MessageLoopError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Application(error) | Self::Source(error) => Some(error),
            Self::DispatcherClosed => None,
        }
    }
}

/// A `WinUI` `Application` and its `DispatcherQueue`-backed message source.
pub struct MessageLoopDispatcherQueue<M, H> {
    application: Application,
    shared: Arc<SharedState<M>>,
    handler: H,
}

impl<M: Send + 'static, H: MessageLoopHandler<M> + 'static> MessageLoopDispatcherQueue<M, H> {
    #[must_use]
    pub fn new(application: Application, handler: H) -> Self {
        Self {
            application,
            shared: Arc::new(SharedState::new()),
            handler,
        }
    }

    #[must_use]
    pub fn sender(&self) -> DispatcherQueueSender<M> {
        DispatcherQueueSender {
            state: Arc::downgrade(&self.shared),
        }
    }

    /// Runs the `WinUI` application until termination is requested or it exits.
    ///
    /// # Errors
    /// Returns an error when application startup or dispatcher source setup fails.
    pub fn run(self) -> Result<(), MessageLoopError> {
        let setup_error = Arc::new(Mutex::new(None));
        let error_for_launch = setup_error.clone();
        let shared = self.shared;
        let handler = self.handler;

        let result = self.application.run(move |application| {
            let callback = Rc::new(CallbackState {
                application: application.clone(),
                shared: shared.clone(),
                handler: RefCell::new(handler),
                initialized: Cell::new(false),
                terminated: Cell::new(false),
                performing: Cell::new(false),
            });
            let callback_for_source = callback.clone();
            let dispatcher = application.dispatcher_queue();
            let source = match dispatcher.create_source(move || callback_for_source.perform()) {
                Ok(source) => source,
                Err(error) => {
                    *error_for_launch
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(MessageLoopError::Source(error));
                    callback.finish();
                    let _ = application.exit();
                    return RuntimeState {
                        source: None,
                        callback,
                    };
                }
            };
            let signaler = match source.signaler() {
                Ok(signaler) => signaler,
                Err(error) => {
                    *error_for_launch
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        Some(MessageLoopError::Source(error));
                    callback.finish();
                    let _ = application.exit();
                    return RuntimeState {
                        source: Some(source),
                        callback,
                    };
                }
            };
            {
                let mut state = shared
                    .queue
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                state.signaler = Some(signaler.clone());
            }
            if !signaler.signal() {
                *error_for_launch
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some(MessageLoopError::DispatcherClosed);
                callback.finish();
                let _ = application.exit();
            }
            RuntimeState {
                source: Some(source),
                callback,
            }
        });

        if let Some(error) = setup_error
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        {
            return Err(error);
        }
        result.map_err(MessageLoopError::Application)
    }
}

pub type DispatcherQueueMessageLoop<M, H> = MessageLoopDispatcherQueue<M, H>;
