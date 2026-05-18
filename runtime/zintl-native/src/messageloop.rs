use std::sync::Arc;

use crate::actor::{MainActor, MainMarker};
#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;

pub struct MainTask<C, M: Send + Sync> {
    pub(crate) f: Box<dyn FnOnce(MainMarker, C) -> () + Send>,
    pub(crate) send_after: Option<M>,
}

pub trait Context<M: Send + Sync>: Clone + Send + Sync + 'static {
    fn perform_main(
        &self,
        f: impl FnOnce(MainMarker, Self) -> () + Send + 'static,
        send_after: Option<M>,
    );
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

    pub fn create_window(
        &self,
        marker: MainMarker,
        on_lifecycle: Arc<dyn Fn(WindowLifecycleEvent) + Send + Sync>,
    ) -> MainActor<Window> {
        self.backend.create_window(marker, on_lifecycle)
    }
}

pub(crate) trait WindowManagerBackend: Send + Sync {
    fn create_window(
        &self,
        marker: MainMarker,
        on_lifecycle: Arc<dyn Fn(WindowLifecycleEvent) + Send + Sync>,
    ) -> MainActor<Window>;
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

    pub fn set_bounds(&self, bounds: Rect) {
        self.backend.set_bounds(bounds);
    }

    pub fn set_size(&self, width: f64, height: f64) {
        self.backend.set_size(width, height);
    }

    pub fn set_position(&self, x: f64, y: f64) {
        self.backend.set_position(x, y);
    }

    pub fn set_commands(
        &self,
        commands: WindowCommandSet,
        on_command: Arc<dyn Fn(WindowCommandEvent) + Send + Sync>,
    ) {
        self.backend.set_commands(commands, on_command);
    }

    #[cfg(feature = "wgpu")]
    pub fn create_wgpu_surface(&self, marker: MainMarker, rect: Rect) -> MainActor<WgpuSurface> {
        MainActor::new(marker, self.backend.create_wgpu_surface(marker, rect))
    }
}

pub(crate) trait WindowBackend: Send + Sync {
    fn show(&self);
    fn set_bounds(&self, bounds: Rect);
    fn set_size(&self, width: f64, height: f64);
    fn set_position(&self, x: f64, y: f64);
    fn set_commands(
        &self,
        commands: WindowCommandSet,
        on_command: Arc<dyn Fn(WindowCommandEvent) + Send + Sync>,
    );

    #[cfg(feature = "wgpu")]
    fn create_wgpu_surface(&self, marker: MainMarker, rect: Rect) -> WgpuSurface;
}

#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WindowCommandSet {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_menu: Option<WindowAppMenu>,
    pub menus: Vec<WindowCommandMenu>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WindowAppMenu {
    pub items: Vec<WindowCommandItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WindowCommandMenu {
    pub title: String,
    pub items: Vec<WindowCommandItem>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct WindowCommandItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<WindowCommandRole>,
    pub key: Option<String>,
    pub modifiers: Vec<WindowCommandModifier>,
    pub enabled: bool,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowCommandModifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum WindowCommandRole {
    About,
    Quit,
}

#[derive(Clone, Debug)]
pub struct WindowCommandEvent {
    pub command_id: String,
}

#[derive(Clone, Debug)]
pub struct WindowLifecycleEvent {
    pub kind: WindowLifecycleEventKind,
}

#[derive(Clone, Debug)]
pub enum WindowLifecycleEventKind {
    Created,
    WillClose,
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
pub(crate) trait WgpuSurfaceBackend: Send + Sync {
    fn surface_target_unsafe(&self) -> wgpu::SurfaceTargetUnsafe;
    fn drawable_size(&self) -> PhysicalSize;
    fn set_rect(&self, rect: Rect);
}

pub enum Event<M: Send + Sync> {
    UserMessage(M),
}

pub trait MessageHandler<M: Send + Sync>: Send + Sync {
    fn on_init(&mut self, _marker: MainMarker, _cx: impl Context<M>) {}
    fn on_event(&mut self, _marker: MainMarker, _cx: impl Context<M>, _event: Event<M>) {}
    fn will_terminate(&mut self, _marker: MainMarker, _cx: impl Context<M>) {}
}
