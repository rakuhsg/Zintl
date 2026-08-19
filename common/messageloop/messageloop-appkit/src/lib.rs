//! A main-thread message loop driven by a Core Foundation run loop source.

#![cfg(target_os = "macos")]

use core_foundation_sys::base::{CFRelease, kCFAllocatorDefault};
pub use core_foundation_sys::runloop::CFRunLoopRef;
use core_foundation_sys::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopGetMain, CFRunLoopRemoveSource, CFRunLoopRun,
    CFRunLoopSourceContext, CFRunLoopSourceCreate, CFRunLoopSourceInvalidate, CFRunLoopSourceRef,
    CFRunLoopSourceSignal, CFRunLoopStop, CFRunLoopWakeUp, kCFRunLoopCommonModes,
};
pub use messageloop_core::{SendError, Sender, SenderResult};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};

/// Returns the process main run loop borrowed for the process lifetime.
#[must_use]
pub fn main_run_loop() -> CFRunLoopRef {
    // SAFETY: Core Foundation returns a borrowed process-owned run loop.
    unsafe { CFRunLoopGetMain() }
}

struct QueueState<M> {
    messages: VecDeque<M>,
    quit_requested: bool,
    closed: bool,
    source: Option<usize>,
}

struct SharedState<M> {
    queue: Mutex<QueueState<M>>,
    run_loop: usize,
}

impl<M> SharedState<M> {
    fn new(run_loop: CFRunLoopRef) -> Self {
        Self {
            queue: Mutex::new(QueueState {
                messages: VecDeque::new(),
                quit_requested: false,
                closed: false,
                source: None,
            }),
            run_loop: run_loop as usize,
        }
    }

    fn signal_locked(&self, state: &QueueState<M>) {
        let Some(source) = state.source else {
            return;
        };

        // SAFETY: Access to `source` is serialized by `queue`. The owning loop
        // clears it under the same lock before invalidating and releasing it.
        unsafe {
            CFRunLoopSourceSignal(source as CFRunLoopSourceRef);
            CFRunLoopWakeUp(self.run_loop as CFRunLoopRef);
        }
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
        self.signal_locked(&state);
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
        shared.signal_locked(&state);
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
    fn perform(&self) {
        // A nested CFRunLoop invocation must not borrow the handler twice. The
        // outer invocation will drain anything queued by the nested loop.
        if self.performing.replace(true) {
            return;
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
        if quit_requested && !self.terminated.replace(true) {
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

            // SAFETY: `run_loop` is the process main run loop and remains valid
            // for the process lifetime. This callback runs on that run loop.
            unsafe { CFRunLoopStop(self.shared.run_loop as CFRunLoopRef) };
        }

        self.performing.set(false);
    }
}

extern "C" fn perform_source<M, H>(info: *const c_void)
where
    M: Send + 'static,
    H: MessageLoopHandler<M>,
{
    let result = catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The source is invalidated before its boxed callback state is
        // dropped, and Core Foundation invokes this function only for it.
        let state = unsafe { &*info.cast::<CallbackState<M, H>>() };
        // SAFETY: These calls only return borrowed, process-owned run loops.
        let is_main = unsafe { CFRunLoopGetCurrent() == CFRunLoopGetMain() };
        if !is_main {
            std::process::abort();
        }
        state.perform();
    }));
    if result.is_err() {
        // Panics must never unwind through the Core Foundation callback ABI.
        std::process::abort();
    }
}

/// A single-source dispatcher owned and run by the process main thread.
pub struct MessageLoopAppkit<M, H> {
    shared: Arc<SharedState<M>>,
    source: CFRunLoopSourceRef,
    _callback: Box<CallbackState<M, H>>,
    main_thread_only: PhantomData<Rc<()>>,
}

impl<M: Send + 'static, H: MessageLoopHandler<M>> MessageLoopAppkit<M, H> {
    /// Creates and registers one source on the supplied main run loop.
    ///
    /// # Safety
    /// `run_loop` must be a valid borrowed `CFRunLoopRef`. This function
    /// verifies that it identifies the process main run loop.
    ///
    /// # Panics
    /// Panics when called off the process main thread or source creation fails.
    #[must_use]
    pub unsafe fn new(run_loop: CFRunLoopRef, handler: H) -> Self {
        // SAFETY: These calls only return borrowed, process-owned run loops.
        let (current, main) = unsafe { (CFRunLoopGetCurrent(), CFRunLoopGetMain()) };
        assert_eq!(
            run_loop, main,
            "AppKit message loop requires the main CFRunLoop"
        );
        assert_eq!(
            current, main,
            "AppKit message loop must be created on the main thread"
        );

        let shared = Arc::new(SharedState::new(run_loop));
        let mut callback = Box::new(CallbackState {
            shared: shared.clone(),
            handler: RefCell::new(handler),
            initialized: Cell::new(false),
            terminated: Cell::new(false),
            performing: Cell::new(false),
        });
        let mut context = CFRunLoopSourceContext {
            version: 0,
            info: ptr::from_mut(callback.as_mut()).cast(),
            retain: None,
            release: None,
            copyDescription: None,
            equal: None,
            hash: None,
            schedule: None,
            cancel: None,
            perform: perform_source::<M, H>,
        };

        // SAFETY: `context.info` points to a stable Box allocation retained by
        // this value until after the source is invalidated.
        let source = unsafe { CFRunLoopSourceCreate(kCFAllocatorDefault, 0, &raw mut context) };
        assert!(!source.is_null(), "failed to create CFRunLoopSource");
        {
            let mut state = shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.source = Some(source as usize);
        }
        // SAFETY: Both references are valid, and this value removes the source
        // before releasing its create ownership.
        unsafe { CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes) };

        Self {
            shared,
            source,
            _callback: callback,
            main_thread_only: PhantomData,
        }
    }

    #[must_use]
    pub fn sender(&self) -> AppkitSender<M> {
        AppkitSender {
            state: Arc::downgrade(&self.shared),
        }
    }

    /// Runs the main `CFRunLoop` until the handler requests termination.
    pub fn run(self) {
        {
            let state = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.shared.signal_locked(&state);
        }
        // SAFETY: Construction proves this value is on the main thread, and
        // its `Rc` marker prevents moving it to another thread.
        unsafe { CFRunLoopRun() };
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

        // SAFETY: This type is main-thread-bound. Clearing `source` while
        // holding the queue lock ensures no sender can signal after this point.
        unsafe {
            CFRunLoopRemoveSource(
                self.shared.run_loop as CFRunLoopRef,
                self.source,
                kCFRunLoopCommonModes,
            );
            CFRunLoopSourceInvalidate(self.source);
            CFRelease(self.source.cast());
        }
    }
}

/// Compatibility name matching the existing native implementation.
pub type AppkitMessageLoop<M, H> = MessageLoopAppkit<M, H>;
