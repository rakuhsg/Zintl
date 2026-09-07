use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::actor::{ActorRef, ActorTree, ApplicationMessage, EventRouteToken, WindowEvent};
use crate::native;
use crate::ui::{CommandError, CommandSet, Window, WindowError};
use zpd_corefoundation::{RunLoop, RunLoopSource, RunLoopSourceSignaler};
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
    signaler: RunLoopSourceSignaler,
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
        self.state.signaler.signal()
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
    run_loop: RunLoop<'static>,
    source: Option<RunLoopSource<'static>>,
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
                zpd_objc::msg_send!(app.as_ptr(), zpd_objc::sel!("setActivationPolicy:"), ((native::NS_APPLICATION_ACTIVATION_POLICY_REGULAR): i64) => ())
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
            let run_loop = RunLoop::current();
            let source = run_loop
                .create_source_with_order(1, {
                    let state = state.clone();
                    move || invoke(&state, ApplicationDelegate::perform)
                })
                .map_err(|_| ApplicationError::NativeCreationFailed)?;
            let signaler = source.signaler();
            Ok(Self {
                _delegate_type: PhantomData,
                scheduler_state: Arc::new(ActiveState {
                    active: AtomicBool::new(true),
                    signaler,
                }),
                run_loop,
                source: Some(source),
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
        self.run_loop
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
        drop(self.source.take());
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
                zpd_objc::msg_send!(app, zpd_objc::sel!("activateIgnoringOtherApps:"), ((true): bool) => ());
                zpd_objc::msg_send!(app, zpd_objc::sel!("run"), () => ());
            }
            ApplicationMessage::Stop => {
                zpd_objc::msg_send!(app, zpd_objc::sel!("stop:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
                let event = zpd_objc::msg_send!(zpd_objc::class!("NSEvent"), zpd_objc::sel!("otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:"), ((native::NS_EVENT_TYPE_APPLICATION_DEFINED): i64, (native::Point::default()): native::Point, (0): u64, (0.0): f64, (0): i64, (zpd_objc::NIL): zpd_objc::Id, (0): i16, (0): i64, (0): i64) => zpd_objc::Id);
                if !event.is_null() {
                    zpd_objc::msg_send!(app, zpd_objc::sel!("postEvent:atStart:"), ((event): zpd_objc::Id, (false): bool) => ());
                }
                RunLoop::main().stop();
                RunLoop::main().wake();
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::RunLoopScheduler;

    #[test]
    fn scheduler_can_cross_threads() {
        // Verifies the scheduler safely crosses the message-loop thread boundary.
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<RunLoopScheduler>();
    }
}
