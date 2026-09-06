use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::actor::{ActorRef, ActorTree, ApplicationMessage, EventRouteToken, WindowEvent};
use crate::native::{self, CFRunLoopSourceContext};
use crate::ui::{CommandError, CommandSet, Window, WindowError};
use zpd_objc::{Id, Strong};

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
    EventHandlerAlreadyRegistered,
}
impl std::fmt::Display for ApplicationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotMainThread => "AppKit must be initialized on the main thread",
            Self::AlreadyInitialized => "AppKit is already initialized",
            Self::NotActive => "AppKit is not active",
            Self::NativeCreationFailed => "AppKit failed to create a native object",
            Self::EventHandlerAlreadyRegistered => {
                "an AppKit window event handler is already registered"
            }
        })
    }
}
impl std::error::Error for ApplicationError {}

/// Active registration for the Application's single Window event callback.
///
/// Dropping this value unregisters the callback. Events emitted before a
/// registration exists or after it is dropped are ignored.
pub struct WindowEventRegistration<'application> {
    tree: ActorTree,
    id: u64,
    _application: PhantomData<&'application ()>,
}

impl Drop for WindowEventRegistration<'_> {
    fn drop(&mut self) {
        self.tree.clear_event_handler(self.id);
    }
}

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

struct ContextSourceCallback<C> {
    context: C,
    perform: fn(&C),
}

unsafe extern "C" fn context_source_perform<C>(info: *mut c_void) {
    if info.is_null() {
        return;
    }
    if catch_unwind(AssertUnwindSafe(|| {
        // SAFETY: ContextRunLoopSource owns this stable allocation while its source is installed.
        let callback = unsafe { &*info.cast::<ContextSourceCallback<C>>() };
        (callback.perform)(&callback.context);
    }))
    .is_err()
    {
        std::process::abort();
    }
}

/// A safe Core Foundation run-loop source owning a concrete callback context.
pub struct ContextRunLoopSource<C> {
    state: Arc<Mutex<SourceState>>,
    _callback: Box<ContextSourceCallback<C>>,
    _main_thread: PhantomData<Rc<()>>,
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

impl RunLoop<'_> {
    /// Creates a source that owns a concrete context and invokes it on this run loop.
    ///
    /// # Errors
    /// Returns an error off the current run loop or when Core Foundation creation fails.
    pub fn create_context_source<C>(
        self,
        context: C,
        perform: fn(&C),
    ) -> Result<ContextRunLoopSource<C>, RunLoopSourceError> {
        if !self.is_current() {
            return Err(RunLoopSourceError::NotCurrent);
        }
        let callback = Box::new(ContextSourceCallback { context, perform });
        let mut context = CFRunLoopSourceContext {
            info: std::ptr::from_ref(callback.as_ref()).cast_mut().cast(),
            perform: Some(context_source_perform::<C>),
            ..Default::default()
        };
        // SAFETY: callback remains stable until the source is removed and invalidated.
        let source =
            unsafe { native::CFRunLoopSourceCreate(std::ptr::null(), 0, &raw mut context) };
        if source.is_null() {
            return Err(RunLoopSourceError::NativeCreationFailed);
        }
        // SAFETY: Both handles are live and owned/borrowed here.
        unsafe { native::CFRunLoopAddSource(self.raw, source, native::kCFRunLoopCommonModes) };
        Ok(ContextRunLoopSource {
            state: Arc::new(Mutex::new(SourceState {
                run_loop: self.raw as usize,
                source: Some(source as usize),
            })),
            _callback: callback,
            _main_thread: PhantomData,
        })
    }
}

fn remove_source(state: &Mutex<SourceState>) {
    let mut state = state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(source) = state.source.take() else {
        return;
    };
    // SAFETY: Source owners are main-thread-only and own the installed Core Foundation source.
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
    unsafe { zpd_objc::get_pointer_ivar(object, c"_zpdCallbacks".as_ptr()) }
}
unsafe extern "C" fn did_launch(object: Id, _: zpd_objc::Sel, _: Id) {
    if let Some(cb) = unsafe { callbacks(object).as_ref() } {
        unsafe { (cb.launch)(cb.state) }
    }
}
unsafe extern "C" fn will_terminate(object: Id, _: zpd_objc::Sel, _: Id) {
    if let Some(cb) = unsafe { callbacks(object).as_ref() } {
        unsafe { (cb.terminate)(cb.state) }
    }
}
unsafe extern "C" fn delegate_dealloc(object: Id, _: zpd_objc::Sel) {
    unsafe { release_callbacks(object) };
    unsafe {
        zpd_objc::msg_send_super!(object, zpd_objc::class!("NSObject"), zpd_objc::sel!("dealloc"), () => ())
    };
}

unsafe fn release_callbacks(object: Id) {
    let cb = unsafe { callbacks(object) };
    if !cb.is_null() {
        // SAFETY: Clearing the ivar transfers the sole callback table allocation to Rust.
        unsafe {
            zpd_objc::set_pointer_ivar(
                object,
                c"_zpdCallbacks".as_ptr(),
                std::ptr::null_mut::<AppCallbacks>(),
            )
        };
        // SAFETY: The delegate owns exactly one callback allocation.
        let cb = unsafe { Box::from_raw(cb) };
        unsafe { (cb.release)(cb.state) };
    }
}
fn delegate_class() -> zpd_objc::Class {
    use std::sync::OnceLock;
    static CLASS: OnceLock<usize> = OnceLock::new();
    *CLASS.get_or_init(|| {
        zpd_objc::decl!(ZpdRustApplicationDelegate: [zpd_objc::class!("NSObject")] {
            fields { _zpdCallbacks: ptr }
            methods {
                "applicationDidFinishLaunching:": "v@:@" => did_launch,
                "applicationWillTerminate:": "v@:@" => will_terminate,
                "dealloc": "v@:" => delegate_dealloc,
            }
        }) as usize
    }) as zpd_objc::Class
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
}
fn shared_application() -> Id {
    unsafe {
        zpd_objc::msg_send!(zpd_objc::class!("NSApplication"), zpd_objc::sel!("sharedApplication"), () => zpd_objc::Id)
    }
}

thread_local! {
    static ROOT_TREE: RefCell<Option<ActorTree>> = const { RefCell::new(None) };
}

fn root_tree(app: &Strong) -> ActorTree {
    ROOT_TREE.with(|tree| {
        let mut tree = tree.borrow_mut();
        tree.get_or_insert_with(|| ActorTree::new(app.clone()))
            .begin_session()
    })
}

pub struct Application<D: ApplicationDelegate> {
    _delegate_type: PhantomData<D>,
    scheduler_state: Arc<ActiveState>,
    source: *mut c_void,
    callback: *mut SourceCallback<'static>,
    delegate: ActorRef,
    actor: ActorRef,
    tree: ActorTree,
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
                zpd_objc::msg_send!(app.as_ptr(), zpd_objc::sel!("setActivationPolicy:"), ((0): i64) => ())
            };
            let tree = root_tree(&app);
            let actor = tree.root();
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
                let object = zpd_objc::msg_send!(zpd_objc::msg_send!(class, zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("init"), () => zpd_objc::Id);
                zpd_objc::set_pointer_ivar(
                    object,
                    c"_zpdCallbacks".as_ptr(),
                    Box::into_raw(callback_table),
                );
                Strong::from_retained(object).ok_or(ApplicationError::NativeCreationFailed)?
            };
            unsafe {
                zpd_objc::msg_send!(actor
                        .with(|id| id)
                        .map_err(|_| ApplicationError::NotActive)?, zpd_objc::sel!("setDelegate:"), ((native_delegate.as_ptr()): zpd_objc::Id) => ())
            };
            let delegate_actor = tree
                .insert_child(&actor, native_delegate)
                .map_err(|_| ApplicationError::NativeCreationFailed)?;
            tree.add_teardown(&delegate_actor, |delegate| unsafe {
                release_callbacks(delegate)
            })
            .map_err(|_| ApplicationError::NativeCreationFailed)?;
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
                _delegate_type: PhantomData,
                scheduler_state: Arc::new(ActiveState {
                    active: AtomicBool::new(true),
                    run_loop: run_loop as usize,
                    source: source as usize,
                }),
                source,
                callback,
                delegate: delegate_actor,
                actor,
                tree,
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
    pub fn create_window(&self) -> Result<Window<'_>, WindowError> {
        self.create_window_with_event_route(None)
    }

    /// Creates a Window whose Actor carries an opaque event route before the
    /// `Created` event is emitted.
    pub fn create_window_with_event_route(
        &self,
        route: Option<EventRouteToken>,
    ) -> Result<Window<'_>, WindowError> {
        Window::new(self, route)
    }
    /// Registers the sole semantic Window event callback for this Application.
    ///
    /// The callback runs synchronously at the AppKit event source. Consumers
    /// should enqueue work instead of invoking re-entrant UI processing.
    pub fn on(
        &self,
        callback: impl FnMut(WindowEvent) + 'static,
    ) -> Result<WindowEventRegistration<'_>, ApplicationError> {
        let id = self
            .tree
            .set_event_handler(callback)
            .map_err(|error| match error {
                crate::actor::ActorError::InvalidHierarchy => {
                    ApplicationError::EventHandlerAlreadyRegistered
                }
                _ => ApplicationError::NotActive,
            })?;
        Ok(WindowEventRegistration {
            tree: self.tree.clone(),
            id,
            _application: PhantomData,
        })
    }
    pub fn set_commands<F>(&self, commands: &CommandSet, callback: F) -> Result<(), CommandError>
    where
        F: FnMut(&str) + 'static,
    {
        crate::ui::commands::install(self, commands, callback)
    }
    pub fn run(&self) -> Result<(), ApplicationError> {
        self.actor
            .send(ApplicationMessage::Run)
            .map_err(|_| ApplicationError::NotActive)
    }
    pub fn stop(&self) -> Result<(), ApplicationError> {
        self.actor
            .send(ApplicationMessage::Stop)
            .map_err(|_| ApplicationError::NotActive)
    }
    pub fn actor_ref(&self) -> ActorRef {
        self.actor.clone()
    }
    pub(crate) fn tree(&self) -> &ActorTree {
        &self.tree
    }
}
impl<D: ApplicationDelegate> Drop for Application<D> {
    fn drop(&mut self) {
        self.scheduler_state.active.store(false, Ordering::Release);
        // SAFETY: Clearing the menu detaches unretained action targets before their owners drop.
        unsafe {
            let _ = self
                .actor
                .with(|app| zpd_objc::msg_send!(app, zpd_objc::sel!("setMainMenu:"), ((zpd_objc::NIL): zpd_objc::Id) => ()));
        }
        let _ = self.actor.with(|app| unsafe {
            zpd_objc::msg_send!(app, zpd_objc::sel!("setDelegate:"), ((zpd_objc::NIL): zpd_objc::Id) => ())
        });
        self.delegate.remove();
        // SAFETY: Application owns the delegate binding, CF source, and callback allocation.
        unsafe {
            native::CFRunLoopRemoveSource(
                self.scheduler_state.run_loop as *mut c_void,
                self.source,
                native::kCFRunLoopCommonModes,
            );
            native::CFRunLoopSourceInvalidate(self.source);
            native::CFRelease(self.source);
            drop(Box::from_raw(self.callback));
        }
        self.tree.end_session();
        INITIALIZED.store(false, Ordering::Release);
    }
}

pub(crate) fn send_application_message(
    actor: &ActorRef,
    message: ApplicationMessage,
) -> Result<(), crate::actor::ActorError> {
    actor.with(|app| unsafe {
        match message {
            ApplicationMessage::Run => {
                zpd_objc::msg_send!(app, zpd_objc::sel!("activateIgnoringOtherApps:"), ((1): i64) => ());
                zpd_objc::msg_send!(app, zpd_objc::sel!("run"), () => ());
            }
            ApplicationMessage::Stop => {
                zpd_objc::msg_send!(app, zpd_objc::sel!("stop:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
                let event = zpd_objc::msg_send!(zpd_objc::class!("NSEvent"), zpd_objc::sel!("otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:"), ((15): u64, (native::Point::default()): native::Point, (0): u64, (0.0): f64, (0): i64, (zpd_objc::NIL): zpd_objc::Id, (0): i16, (0): i64, (0): i64) => zpd_objc::Id);
                if !event.is_null() {
                    zpd_objc::msg_send!(app, zpd_objc::sel!("postEvent:atStart:"), ((event): zpd_objc::Id, (false): bool) => ());
                }
                native::CFRunLoopStop(native::CFRunLoopGetMain());
                native::CFRunLoopWakeUp(native::CFRunLoopGetMain());
            }
        }
    })
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
