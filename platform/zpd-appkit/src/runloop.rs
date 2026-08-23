use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::actor::ActorTree;
use crate::native::{self, CFRunLoopSourceContext, Id, Strong};
use crate::ui::{CommandError, CommandSet, Window, WindowDelegate, WindowError};

static INITIALIZED: AtomicBool = AtomicBool::new(false);

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
    NativeCreationFailed,
}
impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotMainThread => "AppKit must be initialized on the main thread",
            Self::AlreadyInitialized => "AppKit is already initialized",
            Self::NotActive => "AppKit is not active",
            Self::NativeCreationFailed => "AppKit failed to create a native object",
        })
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
        f.write_str(match self {
            Self::NotCurrent => "run-loop sources must be created on their run loop",
            Self::NativeCreationFailed => "AppKit failed to create a run-loop source",
        })
    }
}
impl std::error::Error for RunLoopSourceError {}

struct SourceCallback<'a> {
    callback: RefCell<Box<dyn FnMut() + 'a>>,
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

#[derive(Clone, Copy)]
pub struct RunLoop<'application> {
    raw: *mut c_void,
    _application: PhantomData<&'application ()>,
    _main_thread: PhantomData<Rc<()>>,
}
impl<'application> RunLoop<'application> {
    pub fn is_current(self) -> bool {
        // SAFETY: Core Foundation returns borrowed process run-loop pointers.
        self.raw == unsafe { native::CFRunLoopGetCurrent() }
    }
    pub fn stop(self) {
        // SAFETY: raw is live for the Application borrow.
        unsafe { native::CFRunLoopStop(self.raw) };
    }
    pub fn create_source(
        self,
        callback: impl FnMut() + 'application,
    ) -> Result<RunLoopSource<'application>, RunLoopSourceError> {
        if !self.is_current() {
            return Err(RunLoopSourceError::NotCurrent);
        }
        let callback = Box::new(SourceCallback {
            callback: RefCell::new(Box::new(callback)),
        });
        let mut context = CFRunLoopSourceContext {
            info: std::ptr::from_ref(callback.as_ref()).cast_mut().cast(),
            perform: Some(source_perform),
            ..Default::default()
        };
        // SAFETY: callback remains stable until the source is invalidated.
        let source = unsafe { native::CFRunLoopSourceCreate(std::ptr::null(), 0, &mut context) };
        if source.is_null() {
            return Err(RunLoopSourceError::NativeCreationFailed);
        }
        // SAFETY: Both handles are live and owned/borrowed here.
        unsafe { native::CFRunLoopAddSource(self.raw, source, native::kCFRunLoopCommonModes) };
        Ok(RunLoopSource {
            state: Arc::new(Mutex::new(SourceState {
                run_loop: self.raw as usize,
                source: Some(source as usize),
            })),
            _callback: callback,
            _application: PhantomData,
            _main_thread: PhantomData,
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
            native::CFRunLoopSourceSignal(source as *mut c_void);
            native::CFRunLoopWakeUp(state.run_loop as *mut c_void);
        }
        true
    }
}
pub struct RunLoopSource<'application> {
    state: Arc<Mutex<SourceState>>,
    _callback: Box<SourceCallback<'application>>,
    _application: PhantomData<&'application ()>,
    _main_thread: PhantomData<Rc<()>>,
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
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(source) = state.source.take() else {
            return;
        };
        // SAFETY: This owner is dropped on the source run-loop thread.
        unsafe {
            native::CFRunLoopRemoveSource(
                state.run_loop as *mut c_void,
                source as *mut c_void,
                native::kCFRunLoopCommonModes,
            );
            native::CFRunLoopSourceInvalidate(source as *mut c_void);
            native::CFRelease(source as *const c_void);
        }
    }
}

struct DelegateState<D> {
    delegate: RefCell<D>,
}
fn invoke<D>(state: &Rc<DelegateState<D>>, operation: impl FnOnce(&mut D)) {
    if catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut delegate) = state.delegate.try_borrow_mut() else {
            std::process::abort()
        };
        operation(&mut delegate);
    }))
    .is_err()
    {
        std::process::abort()
    }
}

struct AppCallbacks {
    state: *const c_void,
    launch: unsafe fn(*const c_void),
    terminate: unsafe fn(*const c_void),
    release: unsafe fn(*const c_void),
}
unsafe fn launch<D: ApplicationDelegate>(raw: *const c_void) {
    // SAFETY: raw is a retained Rc pointer owned by AppCallbacks.
    let state = unsafe { Rc::from_raw(raw.cast::<DelegateState<D>>()) };
    invoke(&state, ApplicationDelegate::on_launch);
    let _ = Rc::into_raw(state);
}
unsafe fn terminate<D: ApplicationDelegate>(raw: *const c_void) {
    // SAFETY: raw is a retained Rc pointer owned by AppCallbacks.
    let state = unsafe { Rc::from_raw(raw.cast::<DelegateState<D>>()) };
    invoke(&state, ApplicationDelegate::will_terminate);
    let _ = Rc::into_raw(state);
}
unsafe fn release_state<D>(raw: *const c_void) {
    // SAFETY: This consumes the retained Rc transferred to AppCallbacks.
    unsafe { drop(Rc::from_raw(raw.cast::<DelegateState<D>>())) };
}
unsafe fn callbacks(object: Id) -> *mut AppCallbacks {
    unsafe { native::get_pointer_ivar(object, c"_zpdCallbacks".as_ptr()) }
}
unsafe extern "C" fn did_launch(object: Id, _: native::Sel, _: Id) {
    if let Some(cb) = unsafe { callbacks(object).as_ref() } {
        unsafe { (cb.launch)(cb.state) }
    }
}
unsafe extern "C" fn will_terminate(object: Id, _: native::Sel, _: Id) {
    if let Some(cb) = unsafe { callbacks(object).as_ref() } {
        unsafe { (cb.terminate)(cb.state) }
    }
}
unsafe extern "C" fn delegate_dealloc(object: Id, _: native::Sel) {
    let cb = unsafe { callbacks(object) };
    if !cb.is_null() {
        // SAFETY: The delegate owns exactly one callback allocation.
        let cb = unsafe { Box::from_raw(cb) };
        unsafe { (cb.release)(cb.state) };
    }
    unsafe {
        native::send_super_void(
            object,
            native::class(b"NSObject\0"),
            native::sel(b"dealloc\0"),
        )
    };
}
fn delegate_class() -> native::Class {
    use std::sync::OnceLock;
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| unsafe {
        let class = native::objc_allocateClassPair(
            native::class(b"NSObject\0"),
            c"ZpdRustApplicationDelegate".as_ptr(),
            0,
        );
        assert!(!class.is_null());
        assert!(native::class_addIvar(
            class,
            c"_zpdCallbacks".as_ptr(),
            std::mem::size_of::<Id>(),
            3,
            c"^v".as_ptr()
        ));
        native::add_method(
            class,
            b"applicationDidFinishLaunching:\0",
            did_launch as unsafe extern "C" fn(_, _, _),
            b"v@:@\0",
        );
        native::add_method(
            class,
            b"applicationWillTerminate:\0",
            will_terminate as unsafe extern "C" fn(_, _, _),
            b"v@:@\0",
        );
        native::add_method(
            class,
            b"dealloc\0",
            delegate_dealloc as unsafe extern "C" fn(_, _),
            b"v@:\0",
        );
        native::objc_registerClassPair(class);
        class as usize
    }) as native::Class
}

struct ActiveState {
    active: AtomicBool,
    run_loop: usize,
    source: usize,
}
#[derive(Clone)]
pub struct RunLoopScheduler {
    state: Arc<ActiveState>,
}
impl RunLoopScheduler {
    pub fn schedule(&self) -> bool {
        if !self.state.active.load(Ordering::Acquire) {
            return false;
        }
        // SAFETY: Source signaling and waking are thread-safe Core Foundation operations.
        unsafe {
            native::CFRunLoopSourceSignal(self.state.source as *mut c_void);
            native::CFRunLoopWakeUp(self.state.run_loop as *mut c_void);
        }
        true
    }
    pub fn run(&self) -> Result<(), ApplicationError> {
        if unsafe { native::pthread_main_np() } == 0 {
            return Err(ApplicationError::NotMainThread);
        }
        if !self.state.active.load(Ordering::Acquire) {
            return Err(ApplicationError::NotActive);
        }
        // SAFETY: NSApplication is driven from the process main thread.
        unsafe {
            let app = shared_application();
            native::send_void_i64(app, native::sel(b"activateIgnoringOtherApps:\0"), 1);
            native::send_void(app, native::sel(b"run\0"));
        }
        Ok(())
    }
}
fn shared_application() -> Id {
    unsafe {
        native::send_id(
            native::class(b"NSApplication\0"),
            native::sel(b"sharedApplication\0"),
        )
    }
}

thread_local! {
    static ROOT_TREE: RefCell<Option<ActorTree>> = const { RefCell::new(None) };
}

fn root_tree(app: &Strong) -> ActorTree {
    ROOT_TREE.with(|tree| {
        let mut tree = tree.borrow_mut();
        tree.get_or_insert_with(|| ActorTree::new(app.clone()))
            .clone()
    })
}

pub struct Application<D: ApplicationDelegate> {
    _state: Rc<DelegateState<D>>,
    scheduler_state: Arc<ActiveState>,
    source: *mut c_void,
    callback: *mut SourceCallback<'static>,
    _delegate: Strong,
    app: Strong,
    tree: ActorTree,
    command_objects: RefCell<Vec<Strong>>,
    _main_thread: PhantomData<Rc<()>>,
}
impl<D: ApplicationDelegate> Application<D> {
    pub fn new(delegate: D) -> Result<Self, ApplicationError> {
        if unsafe { native::pthread_main_np() } == 0 {
            return Err(ApplicationError::NotMainThread);
        }
        if INITIALIZED
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ApplicationError::AlreadyInitialized);
        }
        let result = (|| {
            let app = unsafe { Strong::retain(shared_application()) }
                .ok_or(ApplicationError::NativeCreationFailed)?;
            unsafe {
                native::send_void_i64(app.as_ptr(), native::sel(b"setActivationPolicy:\0"), 0)
            };
            let tree = root_tree(&app);
            let state = Rc::new(DelegateState {
                delegate: RefCell::new(delegate),
            });
            let callback_table = Box::new(AppCallbacks {
                state: Rc::into_raw(state.clone()).cast(),
                launch: launch::<D>,
                terminate: terminate::<D>,
                release: release_state::<D>,
            });
            let class = delegate_class();
            let native_delegate = unsafe {
                let object = native::send_id(
                    native::send_id(class, native::sel(b"alloc\0")),
                    native::sel(b"init\0"),
                );
                native::set_pointer_ivar(
                    object,
                    c"_zpdCallbacks".as_ptr(),
                    Box::into_raw(callback_table),
                );
                Strong::from_retained(object).ok_or(ApplicationError::NativeCreationFailed)?
            };
            unsafe {
                native::send_void_id(
                    app.as_ptr(),
                    native::sel(b"setDelegate:\0"),
                    native_delegate.as_ptr(),
                )
            };
            let callback: Box<SourceCallback<'static>> = Box::new(SourceCallback {
                callback: RefCell::new(Box::new({
                    let state = state.clone();
                    move || invoke(&state, ApplicationDelegate::perform)
                })),
            });
            let callback = Box::into_raw(callback);
            let mut context = CFRunLoopSourceContext {
                info: callback.cast(),
                perform: Some(source_perform),
                ..Default::default()
            };
            let source =
                unsafe { native::CFRunLoopSourceCreate(std::ptr::null(), 1, &mut context) };
            if source.is_null() {
                unsafe { drop(Box::from_raw(callback)) };
                return Err(ApplicationError::NativeCreationFailed);
            }
            let run_loop = unsafe { native::CFRunLoopGetCurrent() };
            unsafe { native::CFRunLoopAddSource(run_loop, source, native::kCFRunLoopCommonModes) };
            Ok(Self {
                _state: state,
                scheduler_state: Arc::new(ActiveState {
                    active: AtomicBool::new(true),
                    run_loop: run_loop as usize,
                    source: source as usize,
                }),
                source,
                callback,
                _delegate: native_delegate,
                app,
                tree,
                command_objects: RefCell::new(Vec::new()),
                _main_thread: PhantomData,
            })
        })();
        if result.is_err() {
            INITIALIZED.store(false, Ordering::Release)
        }
        result
    }
    pub fn scheduler(&self) -> RunLoopScheduler {
        RunLoopScheduler {
            state: self.scheduler_state.clone(),
        }
    }
    pub fn run_loop(&self) -> RunLoop<'_> {
        RunLoop {
            raw: self.scheduler_state.run_loop as *mut c_void,
            _application: PhantomData,
            _main_thread: PhantomData,
        }
    }
    pub fn schedule(&self) {
        debug_assert!(self.scheduler().schedule())
    }
    pub fn create_window<W: WindowDelegate>(
        &self,
        delegate: W,
    ) -> Result<Window<'_, W>, WindowError> {
        Window::new(self, delegate)
    }
    pub fn set_commands<F>(&self, commands: &CommandSet, callback: F) -> Result<(), CommandError>
    where
        F: FnMut(&str) + 'static,
    {
        crate::ui::commands::install(self, commands, callback)
    }
    pub fn run(&self) {
        self.scheduler()
            .run()
            .expect("Application can only run while active on main")
    }
    pub fn stop(&self) {
        unsafe {
            native::send_void_id(self.app.as_ptr(), native::sel(b"stop:\0"), native::NIL);
            let event = native::send_application_event(
                native::class(b"NSEvent\0"),
                native::sel(b"otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:\0"),
            );
            if !event.is_null() {
                native::send_void_id_bool(
                    self.app.as_ptr(),
                    native::sel(b"postEvent:atStart:\0"),
                    event,
                    false,
                );
            }
            native::CFRunLoopStop(self.scheduler_state.run_loop as *mut c_void);
            native::CFRunLoopWakeUp(self.scheduler_state.run_loop as *mut c_void);
        }
    }
    pub(crate) fn tree(&self) -> &ActorTree {
        &self.tree
    }
    pub(crate) fn app(&self) -> Id {
        self.app.as_ptr()
    }
    pub(crate) fn replace_command_objects(&self, objects: Vec<Strong>) {
        *self.command_objects.borrow_mut() = objects
    }
}
impl<D: ApplicationDelegate> Drop for Application<D> {
    fn drop(&mut self) {
        self.scheduler_state.active.store(false, Ordering::Release);
        // SAFETY: Clearing the menu detaches unretained action targets before their owners drop.
        unsafe {
            native::send_void_id(
                self.app.as_ptr(),
                native::sel(b"setMainMenu:\0"),
                native::NIL,
            );
        }
        self.command_objects.borrow_mut().clear();
        self.tree.clear();
        // SAFETY: Application owns the delegate binding, CF source, and callback allocation.
        unsafe {
            native::send_void_id(
                self.app.as_ptr(),
                native::sel(b"setDelegate:\0"),
                native::NIL,
            );
            native::CFRunLoopRemoveSource(
                self.scheduler_state.run_loop as *mut c_void,
                self.source,
                native::kCFRunLoopCommonModes,
            );
            native::CFRunLoopSourceInvalidate(self.source);
            native::CFRelease(self.source);
            drop(Box::from_raw(self.callback));
        }
        INITIALIZED.store(false, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::{RunLoopScheduler, RunLoopSourceSignaler};
    #[test]
    fn scheduler_can_cross_threads() {
        // Verifies the scheduler safely crosses the message-loop thread boundary.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RunLoopScheduler>();
    }
    #[test]
    fn signaler_can_cross_threads() {
        // Verifies a run-loop source signaler is thread-safe.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RunLoopSourceSignaler>();
    }
}
