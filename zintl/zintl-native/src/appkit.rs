use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::sync::{Arc, RwLock};

use crossbeam::queue::SegQueue;

use crate::actor::*;
use crate::messageloop::{
    Context, Event, MainTask, MessageHandler, Window, WindowBackend, WindowManager,
    WindowManagerBackend,
};

#[cfg(feature = "wgpu")]
use crate::geometry::{PhysicalSize, Rect};
#[cfg(feature = "wgpu")]
use crate::messageloop::{WgpuSurface, WgpuSurfaceBackend};

mod ffi;

pub struct AppkitContext<M, H: MessageHandler<M>> {
    mesloop: Arc<AppkitMessageLoop<M, H>>,
}

impl<M, H: MessageHandler<M>> Clone for AppkitContext<M, H> {
    fn clone(&self) -> Self {
        AppkitContext {
            mesloop: self.mesloop.clone(),
        }
    }
}

impl<M, H: MessageHandler<M>> AppkitContext<M, H> {
    pub(crate) fn new(mesloop: Arc<AppkitMessageLoop<M, H>>) -> Self {
        AppkitContext { mesloop }
    }
}

impl<M: 'static, H: MessageHandler<M> + 'static> Context<M> for AppkitContext<M, H> {
    fn perform_main(
        &self,
        f: impl FnOnce(MainMarker, Self) -> () + 'static,
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

impl<M, H: MessageHandler<M>> AppkitContext<M, H> {
    fn schedule(&self) {
        // SAFETY: `AppkitMessageLoop::new` initializes the Swift-side run-loop
        // source before any `AppkitContext` can be created.
        unsafe {
            ffi::zintlappkit_schedule();
        }
    }
}

struct AppkitWindowManagerBackend {
    windows: RwLock<Vec<MainActor<Window>>>,
}

impl AppkitWindowManagerBackend {
    fn new() -> Self {
        AppkitWindowManagerBackend {
            windows: RwLock::new(Vec::new()),
        }
    }
}

impl WindowManagerBackend for AppkitWindowManagerBackend {
    fn create_window(&self, marker: MainMarker) -> MainActor<Window> {
        // SAFETY: `marker` witnesses that this code is running on the AppKit
        // main actor, which is required by `zintlappkit_create_window`.
        let ptr = unsafe { ffi::zintlappkit_create_window() };
        let window = MainActor::new(marker, Window::new(Box::new(AppkitWindowBackend { ptr })));

        if let Ok(mut windows) = self.windows.write() {
            windows.push(window.clone());
        }

        window
    }
}

struct AppkitWindowBackend {
    ptr: *const c_void,
}

impl WindowBackend for AppkitWindowBackend {
    fn show(&self) {
        // SAFETY: `Window` is only exposed through `MainActor`, so callers
        // need a `MainMarker` to read it and call AppKit-backed methods.
        unsafe {
            ffi::zintlappkit_show_window(self.ptr);
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

pub struct AppkitMessageLoop<M, H: MessageHandler<M>> {
    initialized: bool,
    handler: RwLock<H>,
    queue: SegQueue<MainTask<AppkitContext<M, H>, M>>,
    window_manager: WindowManager,
    phantom: std::marker::PhantomData<M>,
}

impl<M: 'static, H: MessageHandler<M> + 'static> AppkitMessageLoop<M, H> {
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
        }
    }

    extern "C" fn cb_app_on_init(p_ud: *const c_void) {
        // SAFETY: `p_ud` is the same `Arc<AppkitMessageLoop<_, _>>` raw pointer
        // registered by `new`; Swift stores it unchanged for callback use.
        // `ManuallyDrop` prevents releasing that retained FFI-owned reference.
        let mesloop = ManuallyDrop::new(unsafe { Arc::from_raw(p_ud.cast::<Self>()) });
        let cx = Arc::clone(&*mesloop).context();
        //TODO: unwrap
        let mut handler = mesloop.handler.write().unwrap();
        handler.on_init(cx);
    }
    extern "C" fn cb_app_will_terminate(_p_ud: *const c_void) {}

    fn dispatch_message(self: &Arc<Self>, message: M) {
        let cx = self.clone().context();
        //TODO: unwrap
        let mut handler = self.handler.write().unwrap();
        handler.on_event(cx, Event::UserMessage(message));
    }

    pub fn new(handler: H) -> Arc<Self> {
        let queue = SegQueue::new();
        let mesloop = Arc::new(AppkitMessageLoop {
            initialized: true,
            handler: handler.into(),
            queue,
            window_manager: WindowManager::new(Arc::new(AppkitWindowManagerBackend::new())),
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
