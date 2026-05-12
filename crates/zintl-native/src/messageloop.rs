use std::sync::Arc;

use crate::actor::{MainActor, MainMarker};
#[cfg(feature = "wgpu")]
use crate::geometry::{PhysicalSize, Rect};

pub struct MainTask<C, M> {
    pub(crate) f: Box<dyn FnOnce(MainMarker, C) -> ()>,
    pub(crate) send_after: Option<M>,
}

pub trait Context<M>: Clone + 'static {
    fn perform_main(&self, f: impl FnOnce(MainMarker, Self) -> () + 'static, send_after: Option<M>);
    fn send_message(&self, message: M);
    fn window_manager(&self) -> WindowManager;
}

#[derive(Clone)]
pub struct WindowManager {
    backend: Arc<dyn WindowManagerBackend>,
}

impl WindowManager {
    pub(crate) fn new(backend: Arc<dyn WindowManagerBackend>) -> Self {
        WindowManager { backend }
    }

    pub fn create_window(&self, marker: MainMarker) -> MainActor<Window> {
        self.backend.create_window(marker)
    }
}

pub(crate) trait WindowManagerBackend {
    fn create_window(&self, marker: MainMarker) -> MainActor<Window>;
}

pub struct Window {
    backend: Box<dyn WindowBackend>,
}

impl Window {
    pub(crate) fn new(backend: Box<dyn WindowBackend>) -> Self {
        Window { backend }
    }

    pub fn show(&self) {
        self.backend.show();
    }

    #[cfg(feature = "wgpu")]
    pub fn create_wgpu_surface(&self, marker: MainMarker, rect: Rect) -> MainActor<WgpuSurface> {
        MainActor::new(marker, self.backend.create_wgpu_surface(marker, rect))
    }
}

pub(crate) trait WindowBackend {
    fn show(&self);

    #[cfg(feature = "wgpu")]
    fn create_wgpu_surface(&self, marker: MainMarker, rect: Rect) -> WgpuSurface;
}

#[cfg(feature = "wgpu")]
/// Native view/layer owner used to create a `wgpu::Surface`.
///
/// Keep this value alive until every `wgpu::Surface` created from
/// `surface_target_unsafe` has been dropped.
pub struct WgpuSurface {
    backend: Box<dyn WgpuSurfaceBackend>,
}

#[cfg(feature = "wgpu")]
impl WgpuSurface {
    pub(crate) fn new(backend: Box<dyn WgpuSurfaceBackend>) -> Self {
        WgpuSurface { backend }
    }

    pub fn surface_target_unsafe(&self) -> wgpu::SurfaceTargetUnsafe {
        self.backend.surface_target_unsafe()
    }

    pub fn drawable_size(&self) -> PhysicalSize {
        self.backend.drawable_size()
    }

    pub fn set_rect(&self, rect: Rect) {
        self.backend.set_rect(rect);
    }
}

#[cfg(feature = "wgpu")]
pub(crate) trait WgpuSurfaceBackend {
    fn surface_target_unsafe(&self) -> wgpu::SurfaceTargetUnsafe;
    fn drawable_size(&self) -> PhysicalSize;
    fn set_rect(&self, rect: Rect);
}

pub enum Event<M> {
    UserMessage(M),
}

pub trait MessageHandler<M> {
    fn on_init(&mut self, _cx: impl Context<M>) {}
    fn on_event(&mut self, _cx: impl Context<M>, _event: Event<M>) {}
    fn will_terminate(&mut self, _cx: impl Context<M>) {}
}
