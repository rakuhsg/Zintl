use std::ffi::{CStr, CString, c_char, c_void};
use std::mem::ManuallyDrop;
use std::sync::{Arc, RwLock};

use crossbeam::queue::SegQueue;

use crate::actor::*;
use crate::messageloop::{
    Context, Event, MainTask, MessageHandler, Window, WindowBackend, WindowCommandEvent,
    WindowCommandSet, WindowEvent, WindowId, WindowManager, WindowManagerBackend,
};

#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;
#[cfg(feature = "wgpu")]
use crate::messageloop::{WgpuSurface, WgpuSurfaceBackend};

mod ffi;

pub struct AppkitContext<M: Send + Sync, H: MessageHandler<M>> {
    mesloop: Arc<AppkitMessageLoop<M, H>>,
}

impl<M: Send + Sync, H: MessageHandler<M>> Clone for AppkitContext<M, H> {
    fn clone(&self) -> Self {
        AppkitContext {
            mesloop: self.mesloop.clone(),
        }
    }
}

impl<M: Send + Sync, H: MessageHandler<M>> AppkitContext<M, H> {
    pub(crate) fn new(mesloop: Arc<AppkitMessageLoop<M, H>>) -> Self {
        AppkitContext { mesloop }
    }
}

impl<M: Send + Sync + 'static, H: MessageHandler<M> + 'static> Context<M> for AppkitContext<M, H> {
    fn perform_main(
        &self,
        f: impl FnOnce(MainMarker, Self) -> () + Send + 'static,
        send_after: Option<M>,
    ) {
        self.mesloop.queue.push(MainTask {
            f: Box::new(f),
            send_after,
        });
        self.schedule();
    }

    fn send_message(&self, message: M) {
        self.mesloop.queue.push(MainTask {
            f: Box::new(|_, _| {}),
            send_after: Some(message),
        });
        self.schedule();
    }

    fn window_manager(&self) -> WindowManager {
        self.mesloop.window_manager.clone()
    }
}

impl<M: Send + Sync, H: MessageHandler<M>> AppkitContext<M, H> {
    fn schedule(&self) {
        schedule();
    }
}

fn schedule() {
    // SAFETY: `AppkitMessageLoop::new` initializes the Swift-side run-loop
    // source before any `AppkitContext` can be created.
    unsafe {
        ffi::zintlappkit_schedule();
    }
}

struct AppkitWindowManagerBackend {
    window_events: Arc<SegQueue<(WindowId, WindowEvent)>>,
}

impl AppkitWindowManagerBackend {
    fn new(window_events: Arc<SegQueue<(WindowId, WindowEvent)>>) -> Self {
        AppkitWindowManagerBackend { window_events }
    }
}

impl WindowManagerBackend for AppkitWindowManagerBackend {
    fn create_window(&self, marker: MainMarker, window_id: WindowId) -> MainActor<Window> {
        let event_state = Box::new(AppkitWindowEventState {
            window_id,
            window_events: self.window_events.clone(),
        });
        let callback = ffi::WindowCallback {
            did_create: appkit_window_did_create,
            will_close: appkit_window_will_close,
        };
        // SAFETY: `marker` witnesses that this code is running on the AppKit
        // main actor, and `event_state` stays alive in the backend until
        // the AppKit window is destroyed.
        let user_data = (&*event_state) as *const AppkitWindowEventState;
        let ptr = unsafe { ffi::zintlappkit_create_window(user_data.cast(), &callback) };
        let window = MainActor::new(
            marker,
            Window::new(Box::new(AppkitWindowBackend {
                ptr,
                _event_state: event_state,
            })),
        );
        self.window_events.push((window_id, WindowEvent::Created));
        schedule();
        window
    }
}

struct AppkitWindowBackend {
    ptr: *const c_void,
    _event_state: Box<AppkitWindowEventState>,
}

unsafe impl Send for AppkitWindowBackend {}
unsafe impl Sync for AppkitWindowBackend {}

impl WindowBackend for AppkitWindowBackend {
    fn show(&self) {
        // SAFETY: `Window` is only exposed through `MainActor`, so callers
        // need a `MainMarker` to read it and call AppKit-backed methods.
        unsafe {
            ffi::zintlappkit_show_window(self.ptr);
        }
    }

    fn set_bounds(&self, bounds: Rect) {
        // SAFETY: `Window` is only exposed through `MainActor`, so AppKit
        // mutation happens on the main actor.
        unsafe {
            ffi::zintlappkit_window_set_bounds(self.ptr, bounds);
        }
    }

    fn set_size(&self, width: f64, height: f64) {
        // SAFETY: `Window` is only exposed through `MainActor`, so AppKit
        // mutation happens on the main actor.
        unsafe {
            ffi::zintlappkit_window_set_size(self.ptr, width, height);
        }
    }

    fn set_position(&self, x: f64, y: f64) {
        // SAFETY: `Window` is only exposed through `MainActor`, so AppKit
        // mutation happens on the main actor.
        unsafe {
            ffi::zintlappkit_window_set_position(self.ptr, x, y);
        }
    }

    fn set_commands(
        &self,
        commands: WindowCommandSet,
        on_command: Arc<dyn Fn(WindowCommandEvent) + Send + Sync>,
    ) {
        let commands_json = match serde_json::to_string(&commands) {
            Ok(commands_json) => commands_json,
            Err(error) => {
                eprintln!("failed to encode window commands: {error}");
                return;
            }
        };
        let Ok(commands_json) = CString::new(commands_json) else {
            eprintln!("failed to encode window commands: JSON contains NUL byte");
            return;
        };
        let user_data = Box::into_raw(Box::new(AppkitWindowCommandState { on_command }));

        // SAFETY: `commands_json` is valid for the duration of the call. Swift
        // copies the JSON string and takes ownership of `user_data`, releasing
        // it through `appkit_window_command_release`.
        unsafe {
            ffi::zintlappkit_window_set_commands(
                self.ptr,
                commands_json.as_ptr(),
                user_data.cast(),
                appkit_window_command,
                appkit_window_command_release,
            );
        }
    }

    #[cfg(feature = "wgpu")]
    fn create_wgpu_surface(&self, _marker: MainMarker, rect: Rect) -> WgpuSurface {
        // SAFETY: `Window` is only exposed through `MainActor`, so callers need
        // a `MainMarker` to invoke this AppKit-backed method on the main actor.
        let ptr = unsafe { ffi::zintlappkit_create_wgpu_surface(self.ptr, rect) };
        WgpuSurface::new(Box::new(AppkitWgpuSurfaceBackend { ptr }))
    }
}

struct AppkitWindowCommandState {
    on_command: Arc<dyn Fn(WindowCommandEvent) + Send + Sync>,
}

struct AppkitWindowEventState {
    window_id: WindowId,
    window_events: Arc<SegQueue<(WindowId, WindowEvent)>>,
}

unsafe extern "C" fn appkit_window_command(user_data: *const c_void, command_id: *const c_char) {
    if user_data.is_null() || command_id.is_null() {
        return;
    }

    // SAFETY: Swift passes back the exact user data pointer previously provided
    // to `zintlappkit_window_set_commands`; it remains alive until the paired
    // release callback is invoked.
    let state = unsafe { &*user_data.cast::<AppkitWindowCommandState>() };
    // SAFETY: Swift provides a NUL-terminated UTF-8 command id for this call.
    let command_id = unsafe { CStr::from_ptr(command_id) }
        .to_string_lossy()
        .into_owned();
    (state.on_command)(WindowCommandEvent { command_id });
}

unsafe extern "C" fn appkit_window_did_create(user_data: *const c_void) {
    let _ = user_data;
}

unsafe extern "C" fn appkit_window_will_close(user_data: *const c_void) {
    if user_data.is_null() {
        return;
    }

    // SAFETY: Swift passes back the event user data pointer provided to
    // `zintlappkit_create_window`; the backend owns it for the window lifetime.
    let state = unsafe { &*user_data.cast::<AppkitWindowEventState>() };
    state
        .window_events
        .push((state.window_id, WindowEvent::WillClose));
    schedule();
}

unsafe extern "C" fn appkit_window_command_release(user_data: *const c_void) {
    if user_data.is_null() {
        return;
    }

    // SAFETY: `user_data` was allocated by `Box::into_raw` in
    // `set_commands`, and Swift calls this release callback exactly once for
    // each stored callback state.
    unsafe {
        drop(Box::from_raw(
            user_data.cast_mut().cast::<AppkitWindowCommandState>(),
        ));
    }
}

impl Drop for AppkitWindowBackend {
    fn drop(&mut self) {
        // SAFETY: Windows are retained by `AppkitWindowManager` and dropped when
        // the AppKit message loop is torn down on the main thread.
        unsafe {
            ffi::zintlappkit_destroy_window(self.ptr);
        }
    }
}

#[cfg(feature = "wgpu")]
struct AppkitWgpuSurfaceBackend {
    ptr: *const c_void,
}

#[cfg(feature = "wgpu")]
unsafe impl Send for AppkitWgpuSurfaceBackend {}
#[cfg(feature = "wgpu")]
unsafe impl Sync for AppkitWgpuSurfaceBackend {}

#[cfg(feature = "wgpu")]
impl WgpuSurfaceBackend for AppkitWgpuSurfaceBackend {
    fn surface_target_unsafe(&self) -> wgpu::SurfaceTargetUnsafe {
        // SAFETY: `self.ptr` is retained by this backend and remains valid until
        // `Drop`; Swift keeps the CAMetalLayer alive for the same lifetime.
        let layer = unsafe { ffi::zintlappkit_wgpu_surface_metal_layer(self.ptr) };
        wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer)
    }

    fn drawable_size(&self) -> PhysicalSize {
        // SAFETY: `self.ptr` is retained by this backend and remains valid until
        // `Drop`.
        unsafe { ffi::wgpu_surface_drawable_size(self.ptr) }
    }

    fn set_rect(&self, rect: Rect) {
        // SAFETY: `self.ptr` is retained by this backend and AppKit mutation is
        // reached through the main-actor `WgpuSurface` wrapper.
        unsafe {
            ffi::zintlappkit_wgpu_surface_set_rect(self.ptr, rect);
        }
    }
}

#[cfg(feature = "wgpu")]
impl Drop for AppkitWgpuSurfaceBackend {
    fn drop(&mut self) {
        // SAFETY: The pointer was returned retained by Swift and is released
        // exactly once here when the Rust owner is dropped.
        unsafe {
            ffi::zintlappkit_destroy_wgpu_surface(self.ptr);
        }
    }
}

pub struct AppkitMessageLoop<M: Send + Sync, H: MessageHandler<M>> {
    initialized: bool,
    handler: RwLock<H>,
    queue: SegQueue<MainTask<AppkitContext<M, H>, M>>,
    window_events: Arc<SegQueue<(WindowId, WindowEvent)>>,
    window_manager: WindowManager,
    phantom: std::marker::PhantomData<M>,
}

impl<M: Send + Sync + 'static, H: MessageHandler<M> + 'static> AppkitMessageLoop<M, H> {
    extern "C" fn cb_perform(s_ptr: *const c_void) {
        // SAFETY: `s_ptr` is the user-data pointer passed to `zintlappkit_init`,
        // created by `Arc::into_raw` in `new`. `ManuallyDrop` keeps the FFI-owned
        // strong reference alive after this temporary `Arc` borrow.
        let mesloop = ManuallyDrop::new(unsafe { Arc::from_raw(s_ptr.cast::<Self>()) });
        while let Some(task) = mesloop.queue.pop() {
            let cx = Arc::clone(&*mesloop).context();
            (task.f)(MainMarker::new(), cx);

            if let Some(message) = task.send_after {
                mesloop.dispatch_message(message);
            }

            mesloop.dispatch_pending_window_events();
        }

        mesloop.dispatch_pending_window_events();
    }

    extern "C" fn cb_app_on_init(p_ud: *const c_void) {
        // SAFETY: `p_ud` is the same `Arc<AppkitMessageLoop<_, _>>` raw pointer
        // registered by `new`; Swift stores it unchanged for callback use.
        // `ManuallyDrop` prevents releasing that retained FFI-owned reference.
        let mesloop = ManuallyDrop::new(unsafe { Arc::from_raw(p_ud.cast::<Self>()) });
        let cx = Arc::clone(&*mesloop).context();
        //TODO: unwrap
        let mut handler = mesloop.handler.write().unwrap();
        handler.on_init(MainMarker::new(), cx);
    }
    extern "C" fn cb_app_will_terminate(p_ud: *const c_void) {
        // SAFETY: `p_ud` is the same `Arc<AppkitMessageLoop<_, _>>` raw pointer
        // registered by `new`; Swift stores it unchanged for callback use.
        // `ManuallyDrop` prevents releasing that retained FFI-owned reference.
        let mesloop = ManuallyDrop::new(unsafe { Arc::from_raw(p_ud.cast::<Self>()) });
        let cx = Arc::clone(&*mesloop).context();
        //TODO: unwrap
        let mut handler = mesloop.handler.write().unwrap();
        handler.will_terminate(MainMarker::new(), cx);
    }

    fn dispatch_message(self: &Arc<Self>, message: M) {
        let cx = self.clone().context();
        //TODO: unwrap
        let mut handler = self.handler.write().unwrap();
        handler.on_event(MainMarker::new(), cx, Event::UserMessage(message));
    }

    pub fn new(handler: H) -> Arc<Self> {
        let queue = SegQueue::new();
        let window_events = Arc::new(SegQueue::new());
        let window_manager = WindowManager::new(Arc::new(AppkitWindowManagerBackend::new(
            window_events.clone(),
        )));
        let mesloop = Arc::new(AppkitMessageLoop {
            initialized: true,
            handler: handler.into(),
            queue,
            window_events,
            window_manager,
            phantom: std::marker::PhantomData,
        });

        let p_ud = Arc::into_raw(mesloop.clone());
        let cb = ffi::AppCallback {
            on_init: Self::cb_app_on_init,
            perform: Self::cb_perform,
            will_terminate: Self::cb_app_will_terminate,
        };
        // SAFETY: `p_ud` is a stable `Arc::into_raw` pointer for FFI user data.
        // `cb` is live for this call, and Swift copies it before returning.
        unsafe { ffi::zintlappkit_init(p_ud as *const c_void, &cb) };

        mesloop
    }

    fn context(self: Arc<Self>) -> AppkitContext<M, H> {
        AppkitContext::new(self.clone())
    }

    fn dispatch_window_event(self: &Arc<Self>, window_id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::WillClose) {
            self.window_manager.remove_window(window_id);
        }

        let cx = self.clone().context();
        //TODO: unwrap
        let mut handler = self.handler.write().unwrap();
        handler.on_event(
            MainMarker::new(),
            cx,
            Event::WindowEvent { window_id, event },
        );
    }

    fn dispatch_pending_window_events(self: &Arc<Self>) {
        while let Some((window_id, event)) = self.window_events.pop() {
            self.dispatch_window_event(window_id, event);
        }
    }

    pub fn run(&self) {
        if self.initialized {
            // SAFETY: `new` called `zintlappkit_init`, installing the AppKit
            // singleton and run-loop source required by `zintlappkit_run`.
            unsafe {
                ffi::zintlappkit_run();
            }
        }
    }
}
