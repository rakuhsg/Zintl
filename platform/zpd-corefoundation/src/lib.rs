//! Safe wrappers for Core Foundation facilities used by ZPD.

use std::cell::RefCell;
use std::ffi::{c_long, c_void};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

type Integer = c_long;
type Boolean = i8;

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopWakeUp(run_loop: *mut c_void);
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopSourceCreate(
        allocator: *const c_void,
        order: Integer,
        context: *mut RunLoopSourceContext,
    ) -> *mut c_void;
    fn CFRunLoopSourceSignal(source: *mut c_void);
    fn CFRunLoopSourceInvalidate(source: *mut c_void);
    fn CFRelease(value: *const c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

#[repr(C)]
struct RunLoopSourceContext {
    version: Integer,
    info: *mut c_void,
    retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    release: Option<unsafe extern "C" fn(*const c_void)>,
    copy_description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> Boolean>,
    hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    schedule: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    cancel: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    perform: Option<unsafe extern "C" fn(*mut c_void)>,
}

impl Default for RunLoopSourceContext {
    fn default() -> Self {
        // SAFETY: A zeroed Core Foundation source context is its documented default.
        unsafe { std::mem::zeroed() }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunLoopSourceError {
    NotCurrent,
    NativeCreationFailed,
}

impl std::fmt::Display for RunLoopSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotCurrent => "run-loop sources must be created on their run loop",
            Self::NativeCreationFailed => "Core Foundation failed to create a run-loop source",
        })
    }
}

impl std::error::Error for RunLoopSourceError {}

struct SourceCallback<'callback> {
    callback: RefCell<Box<dyn FnMut() + 'callback>>,
}

unsafe extern "C" fn source_perform(info: *mut c_void) {
    if info.is_null() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: The source owner keeps this allocation alive during callbacks.
        let state = unsafe { &*info.cast::<SourceCallback<'_>>() };
        let Ok(mut callback) = state.callback.try_borrow_mut() else {
            std::process::abort();
        };
        callback();
    }))
    .is_err()
    {
        std::process::abort();
    }
}

/// A borrowed Core Foundation run loop.
#[derive(Clone, Copy)]
pub struct RunLoop<'run_loop> {
    raw: *mut c_void,
    _lifetime: PhantomData<&'run_loop ()>,
    _thread: PhantomData<Rc<()>>,
}

impl RunLoop<'static> {
    /// Returns the calling thread's run loop.
    pub fn current() -> Self {
        // SAFETY: Core Foundation returns a borrowed process run-loop pointer.
        let raw = unsafe { CFRunLoopGetCurrent() };
        Self {
            raw,
            _lifetime: PhantomData,
            _thread: PhantomData,
        }
    }

    /// Returns the process's main run loop.
    pub fn main() -> Self {
        // SAFETY: Core Foundation returns a borrowed process run-loop pointer.
        let raw = unsafe { CFRunLoopGetMain() };
        Self {
            raw,
            _lifetime: PhantomData,
            _thread: PhantomData,
        }
    }
}

impl RunLoop<'_> {
    pub fn is_current(self) -> bool {
        // SAFETY: Core Foundation returns a borrowed process run-loop pointer.
        self.raw == unsafe { CFRunLoopGetCurrent() }
    }

    pub fn stop(self) {
        // SAFETY: Core Foundation run-loop pointers remain valid for their owning thread.
        unsafe { CFRunLoopStop(self.raw) };
    }

    pub fn wake(self) {
        // SAFETY: Waking a live Core Foundation run loop is thread-safe.
        unsafe { CFRunLoopWakeUp(self.raw) };
    }

    pub fn create_source<'callback>(
        self,
        callback: impl FnMut() + 'callback,
    ) -> Result<RunLoopSource<'callback>, RunLoopSourceError> {
        self.create_source_with_order(0, callback)
    }

    /// Creates a source with an explicit Core Foundation ordering value.
    pub fn create_source_with_order<'callback>(
        self,
        order: Integer,
        callback: impl FnMut() + 'callback,
    ) -> Result<RunLoopSource<'callback>, RunLoopSourceError> {
        if !self.is_current() {
            return Err(RunLoopSourceError::NotCurrent);
        }
        let callback = Box::new(SourceCallback {
            callback: RefCell::new(Box::new(callback)),
        });
        let mut context = RunLoopSourceContext {
            info: std::ptr::from_ref(callback.as_ref()).cast_mut().cast(),
            perform: Some(source_perform),
            ..Default::default()
        };
        // SAFETY: callback remains stable until the source is invalidated.
        let source = unsafe { CFRunLoopSourceCreate(std::ptr::null(), order, &mut context) };
        if source.is_null() {
            return Err(RunLoopSourceError::NativeCreationFailed);
        }
        // SAFETY: Both handles are live and owned or borrowed here.
        unsafe { CFRunLoopAddSource(self.raw, source, kCFRunLoopCommonModes) };
        Ok(RunLoopSource {
            state: Arc::new(Mutex::new(SourceState {
                run_loop: self.raw as usize,
                source: Some(source as usize),
            })),
            _callback: callback,
            _thread: PhantomData,
        })
    }

    /// Creates a source that owns a concrete context and invokes it on this run loop.
    pub fn create_context_source<C>(
        self,
        context: C,
        perform: fn(&C),
    ) -> Result<ContextRunLoopSource<C>, RunLoopSourceError> {
        if !self.is_current() {
            return Err(RunLoopSourceError::NotCurrent);
        }
        let callback = Box::new(ContextSourceCallback { context, perform });
        let mut context = RunLoopSourceContext {
            info: std::ptr::from_ref(callback.as_ref()).cast_mut().cast(),
            perform: Some(context_source_perform::<C>),
            ..Default::default()
        };
        // SAFETY: callback remains stable until the source is removed and invalidated.
        let source = unsafe { CFRunLoopSourceCreate(std::ptr::null(), 0, &raw mut context) };
        if source.is_null() {
            return Err(RunLoopSourceError::NativeCreationFailed);
        }
        // SAFETY: Both handles are live and owned or borrowed here.
        unsafe { CFRunLoopAddSource(self.raw, source, kCFRunLoopCommonModes) };
        Ok(ContextRunLoopSource {
            state: Arc::new(Mutex::new(SourceState {
                run_loop: self.raw as usize,
                source: Some(source as usize),
            })),
            _callback: callback,
            _thread: PhantomData,
        })
    }
}

struct SourceState {
    run_loop: usize,
    source: Option<usize>,
}

#[derive(Clone)]
pub struct RunLoopSourceSignaler {
    state: Arc<Mutex<SourceState>>,
}

impl RunLoopSourceSignaler {
    pub fn signal(&self) -> bool {
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(source) = state.source else {
            return false;
        };
        // SAFETY: Destruction clears source while holding the same lock.
        unsafe {
            CFRunLoopSourceSignal(source as *mut c_void);
            CFRunLoopWakeUp(state.run_loop as *mut c_void);
        }
        true
    }
}

pub struct RunLoopSource<'callback> {
    state: Arc<Mutex<SourceState>>,
    _callback: Box<SourceCallback<'callback>>,
    _thread: PhantomData<Rc<()>>,
}

impl RunLoopSource<'_> {
    pub fn signaler(&self) -> RunLoopSourceSignaler {
        RunLoopSourceSignaler {
            state: self.state.clone(),
        }
    }
}

impl Drop for RunLoopSource<'_> {
    fn drop(&mut self) {
        remove_source(&self.state);
    }
}

struct ContextSourceCallback<C> {
    context: C,
    perform: fn(&C),
}

unsafe extern "C" fn context_source_perform<C>(info: *mut c_void) {
    if info.is_null() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ContextRunLoopSource owns this allocation while its source is installed.
        let callback = unsafe { &*info.cast::<ContextSourceCallback<C>>() };
        (callback.perform)(&callback.context);
    }))
    .is_err()
    {
        std::process::abort();
    }
}

/// A run-loop source owning a concrete callback context.
pub struct ContextRunLoopSource<C> {
    state: Arc<Mutex<SourceState>>,
    _callback: Box<ContextSourceCallback<C>>,
    _thread: PhantomData<Rc<()>>,
}

impl<C> ContextRunLoopSource<C> {
    pub fn signaler(&self) -> RunLoopSourceSignaler {
        RunLoopSourceSignaler {
            state: self.state.clone(),
        }
    }
}

impl<C> Drop for ContextRunLoopSource<C> {
    fn drop(&mut self) {
        remove_source(&self.state);
    }
}

fn remove_source(state: &Mutex<SourceState>) {
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(source) = state.source.take() else {
        return;
    };
    // SAFETY: Source owners are thread-bound and own the installed Core Foundation source.
    unsafe {
        CFRunLoopRemoveSource(
            state.run_loop as *mut c_void,
            source as *mut c_void,
            kCFRunLoopCommonModes,
        );
        CFRunLoopSourceInvalidate(source as *mut c_void);
        CFRelease(source as *const c_void);
    }
}

#[cfg(test)]
mod tests {
    use super::RunLoopSourceSignaler;

    #[test]
    fn signaler_can_cross_threads() {
        // Verifies that a run-loop source signaler is thread-safe.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RunLoopSourceSignaler>();
    }
}
