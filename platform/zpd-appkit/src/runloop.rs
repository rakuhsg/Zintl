use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::ffi;
use crate::ui::{CommandError, CommandSet, Window, WindowDelegate, WindowError};

static INITIALIZED: AtomicBool = AtomicBool::new(false);

/// Receives application lifecycle notifications on the AppKit main thread.
pub trait ApplicationDelegate: 'static {
    fn on_launch(&mut self) {}
    fn perform(&mut self) {}
    fn will_terminate(&mut self) {}
}

impl ApplicationDelegate for () {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationError {
    NotMainThread,
    AlreadyInitialized,
    NotActive,
}

impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotMainThread => write!(f, "AppKit must be initialized on the main thread"),
            Self::AlreadyInitialized => write!(f, "AppKit is already initialized"),
            Self::NotActive => write!(f, "AppKit is not active"),
        }
    }
}

impl std::error::Error for ApplicationError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunLoopSourceError {
    NotCurrent,
    NativeCreationFailed,
}

impl std::fmt::Display for RunLoopSourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotCurrent => write!(f, "run-loop sources must be created on their run loop"),
            Self::NativeCreationFailed => write!(f, "AppKit failed to create a run-loop source"),
        }
    }
}

impl std::error::Error for RunLoopSourceError {}

/// A borrowed, main-thread-bound application run loop.
#[derive(Clone, Copy)]
pub struct RunLoop<'application> {
    raw: NonNull<c_void>,
    _application: PhantomData<&'application ()>,
    _main_thread: PhantomData<Rc<()>>,
}

impl<'application> RunLoop<'application> {
    pub(crate) unsafe fn from_raw(raw: *const c_void) -> Self {
        Self {
            raw: NonNull::new(raw.cast_mut()).expect("AppKit returned a null run loop"),
            _application: PhantomData,
            _main_thread: PhantomData,
        }
    }

    #[must_use]
    pub fn is_current(self) -> bool {
        // SAFETY: `raw` identifies the live run loop borrowed from Application.
        unsafe { ffi::zintlappkit_run_loop_is_current(self.raw.as_ptr()) }
    }

    pub fn stop(self) {
        // SAFETY: `raw` identifies the live run loop borrowed from Application.
        unsafe { ffi::zintlappkit_run_loop_stop(self.raw.as_ptr()) };
    }

    pub fn create_source(
        self,
        callback: impl FnMut() + 'application,
    ) -> Result<RunLoopSource<'application>, RunLoopSourceError> {
        if !self.is_current() {
            return Err(RunLoopSourceError::NotCurrent);
        }

        let callback: Box<RunLoopSourceCallback<'application>> = Box::new(RunLoopSourceCallback {
            callback: RefCell::new(Box::new(callback)),
        });
        // SAFETY: The callback allocation remains stable until the native
        // source is destroyed by RunLoopSource::drop.
        let raw = unsafe {
            ffi::zintlappkit_run_loop_source_create(
                self.raw.as_ptr(),
                std::ptr::from_ref(callback.as_ref()).cast(),
                perform_run_loop_source,
            )
        };
        let raw = NonNull::new(raw.cast_mut()).ok_or(RunLoopSourceError::NativeCreationFailed)?;
        Ok(RunLoopSource {
            state: Arc::new(Mutex::new(RunLoopSourceState {
                raw: Some(raw.as_ptr() as usize),
            })),
            _callback: callback,
            _application: PhantomData,
            _main_thread: PhantomData,
        })
    }
}

struct RunLoopSourceCallback<'callback> {
    callback: RefCell<Box<dyn FnMut() + 'callback>>,
}

unsafe extern "C" fn perform_run_loop_source(user_data: *const c_void) {
    abort_on_panic(|| {
        // SAFETY: RunLoopSource owns this stable callback allocation until it
        // first destroys the native source and prevents further callbacks.
        let callback = unsafe { &*user_data.cast::<RunLoopSourceCallback<'_>>() };
        let Ok(mut callback) = callback.callback.try_borrow_mut() else {
            std::process::abort();
        };
        callback();
    });
}

struct RunLoopSourceState {
    raw: Option<usize>,
}

/// A thread-safe handle that signals one application run-loop source.
#[derive(Clone)]
pub struct RunLoopSourceSignaler {
    state: Arc<Mutex<RunLoopSourceState>>,
}

impl RunLoopSourceSignaler {
    /// Signals and wakes the source's run loop.
    ///
    /// Returns `false` after the owning source has been dropped.
    pub fn signal(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(raw) = state.raw else {
            return false;
        };
        // SAFETY: Source destruction takes this same lock and clears `raw`
        // before releasing the native object.
        unsafe { ffi::zintlappkit_run_loop_source_signal(raw as *const c_void) };
        true
    }
}

/// Owns a source registered on an application run loop.
pub struct RunLoopSource<'application> {
    state: Arc<Mutex<RunLoopSourceState>>,
    _callback: Box<RunLoopSourceCallback<'application>>,
    _application: PhantomData<&'application ()>,
    _main_thread: PhantomData<Rc<()>>,
}

impl RunLoopSource<'_> {
    #[must_use]
    pub fn signaler(&self) -> RunLoopSourceSignaler {
        RunLoopSourceSignaler {
            state: self.state.clone(),
        }
    }
}

impl Drop for RunLoopSource<'_> {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(raw) = state.raw.take() else {
            return;
        };
        // SAFETY: This value owns the native source and is main-thread-bound.
        // Holding the signal lock prevents concurrent access during release.
        unsafe { ffi::zintlappkit_run_loop_source_destroy(raw as *const c_void) };
    }
}

struct DelegateState<D> {
    delegate: RefCell<D>,
}

unsafe fn clone_state<D>(user_data: *const c_void) -> Option<Rc<DelegateState<D>>> {
    let state = user_data.cast::<DelegateState<D>>();
    if state.is_null() {
        return None;
    }

    // SAFETY: Swift holds the strong reference transferred by
    // `Application::new` while callbacks remain registered.
    unsafe { Rc::increment_strong_count(state) };
    // SAFETY: The increment above created the strong reference returned here.
    Some(unsafe { Rc::from_raw(state) })
}

fn abort_on_panic(f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        std::process::abort();
    }
}

unsafe fn invoke_delegate<D: ApplicationDelegate>(
    user_data: *const c_void,
    f: impl FnOnce(&mut D),
) {
    // SAFETY: The callback is only installed with an Rc-backed delegate state.
    let Some(state) = (unsafe { clone_state::<D>(user_data) }) else {
        return;
    };

    abort_on_panic(|| {
        let Ok(mut delegate) = state.delegate.try_borrow_mut() else {
            std::process::abort();
        };
        f(&mut delegate);
    });
}

unsafe extern "C" fn on_launch<D: ApplicationDelegate>(user_data: *const c_void) {
    // SAFETY: This callback receives the pointer registered for
    // `DelegateState<D>` by `Application::new`.
    unsafe { invoke_delegate::<D>(user_data, ApplicationDelegate::on_launch) };
}

unsafe extern "C" fn perform<D: ApplicationDelegate>(user_data: *const c_void) {
    // SAFETY: This callback receives the pointer registered for
    // `DelegateState<D>` by `Application::new`.
    unsafe { invoke_delegate::<D>(user_data, ApplicationDelegate::perform) };
}

unsafe extern "C" fn will_terminate<D: ApplicationDelegate>(user_data: *const c_void) {
    // SAFETY: This callback receives the pointer registered for
    // `DelegateState<D>` by `Application::new`.
    unsafe { invoke_delegate::<D>(user_data, ApplicationDelegate::will_terminate) };
}

struct ActiveState {
    active: AtomicBool,
}

/// A thread-safe handle used to wake the AppKit run loop.
#[derive(Clone)]
pub struct RunLoopScheduler {
    state: Arc<ActiveState>,
}

impl RunLoopScheduler {
    /// Signals the AppKit run loop.
    ///
    /// Returns `false` when the owning [`Application`] has already been
    /// dropped.
    pub fn schedule(&self) -> bool {
        if !self.state.active.load(Ordering::Acquire) {
            return false;
        }

        // SAFETY: The application is initialized while `active` is true.
        // Swift accepts scheduling from any thread and forwards it to the
        // AppKit main actor.
        unsafe { ffi::zintlappkit_schedule() };
        true
    }

    /// Runs the AppKit event loop on the process main thread.
    pub fn run(&self) -> Result<(), ApplicationError> {
        // SAFETY: `pthread_main_np` only queries the calling thread.
        if unsafe { ffi::pthread_main_np() } == 0 {
            return Err(ApplicationError::NotMainThread);
        }
        if !self.state.active.load(Ordering::Acquire) {
            return Err(ApplicationError::NotActive);
        }

        // SAFETY: The active state proves that the owning application is
        // initialized, and the check above proves this is the main thread.
        unsafe { ffi::zintlappkit_run() };
        Ok(())
    }
}

/// Owns the process-wide AppKit application integration.
///
/// This type is main-thread-bound and only one value may exist at a time.
pub struct Application<D: ApplicationDelegate> {
    ffi_state: NonNull<DelegateState<D>>,
    _state: Rc<DelegateState<D>>,
    scheduler_state: Arc<ActiveState>,
    _main_thread: PhantomData<Rc<()>>,
}

impl<D: ApplicationDelegate> Application<D> {
    pub fn new(delegate: D) -> Result<Self, ApplicationError> {
        // SAFETY: `pthread_main_np` only queries the calling thread.
        if unsafe { ffi::pthread_main_np() } == 0 {
            return Err(ApplicationError::NotMainThread);
        }

        if INITIALIZED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ApplicationError::AlreadyInitialized);
        }

        let state = Rc::new(DelegateState {
            delegate: RefCell::new(delegate),
        });
        let ffi_state = NonNull::new(Rc::into_raw(state.clone()).cast_mut())
            .expect("Rc never produces a null pointer");
        let callbacks = ffi::AppCallback {
            on_launch: on_launch::<D>,
            perform: perform::<D>,
            will_terminate: will_terminate::<D>,
        };

        // SAFETY: `state` is a stable heap allocation and remains alive until
        // `Drop`. Swift copies the callback table before this call returns.
        unsafe { ffi::zintlappkit_init(ffi_state.as_ptr().cast(), &callbacks) };

        Ok(Self {
            ffi_state,
            _state: state,
            scheduler_state: Arc::new(ActiveState {
                active: AtomicBool::new(true),
            }),
            _main_thread: PhantomData,
        })
    }

    pub fn scheduler(&self) -> RunLoopScheduler {
        RunLoopScheduler {
            state: self.scheduler_state.clone(),
        }
    }

    #[must_use]
    pub fn run_loop(&self) -> RunLoop<'_> {
        // SAFETY: A live Application owns the non-null run-loop wrapper for
        // the duration of this borrow.
        unsafe { RunLoop::from_raw(ffi::zintlappkit_application_run_loop()) }
    }

    /// fire perform callback
    pub fn schedule(&self) {
        let scheduled = self.scheduler().schedule();
        // TODO
        debug_assert!(scheduled);
    }

    pub fn create_window<W: WindowDelegate>(
        &self,
        delegate: W,
    ) -> Result<Window<'_, W>, WindowError> {
        Window::new(delegate)
    }

    pub fn set_commands<F>(&self, commands: &CommandSet, callback: F) -> Result<(), CommandError>
    where
        F: FnMut(&str) + 'static,
    {
        crate::ui::commands::install(commands, callback)
    }

    /// Runs the AppKit event loop until the application terminates.
    pub fn run(&self) {
        self.scheduler()
            .run()
            .expect("Application can only run while active on the main thread");
    }

    /// Stops the AppKit event loop after the current event finishes.
    pub fn stop(&self) {
        // SAFETY: Application is active and this type is main-thread-bound.
        unsafe { ffi::zintlappkit_stop() };
    }
}

impl<D: ApplicationDelegate> Drop for Application<D> {
    fn drop(&mut self) {
        self.scheduler_state.active.store(false, Ordering::Release);

        // SAFETY: This value can only be dropped on the main thread. Destroying
        // the support state detaches the Swift application delegate before the
        // callback allocation is released.
        unsafe {
            ffi::zintlappkit_destroy();
            drop(Rc::from_raw(self.ffi_state.as_ptr()));
        }
        INITIALIZED.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::{RunLoopScheduler, RunLoopSourceSignaler};

    #[test]
    fn scheduler_can_cross_threads() {
        // Verifies the scheduler can safely wake AppKit from worker threads.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RunLoopScheduler>();
    }

    #[test]
    fn source_signaler_can_cross_threads() {
        // Verifies source signaling remains safe across the message-loop boundary.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RunLoopSourceSignaler>();
    }
}
