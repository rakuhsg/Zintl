use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use crate::actor::{MainActor, MainActorRef, MainMarker};
#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;

pub type WindowId = u32;

pub type WindowResult<T> = Result<T, WindowError>;

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
    next_window_id: Arc<AtomicU32>,
    windows: Arc<RwLock<HashMap<WindowId, MainActor<Window>>>>,
}

impl WindowManager {
    pub(crate) fn new(backend: Arc<dyn WindowManagerBackend>) -> Self {
        WindowManager {
            backend,
            next_window_id: Arc::new(AtomicU32::new(0)),
            windows: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn create_window(&self, marker: MainMarker) -> (WindowId, MainActorRef<Window>) {
        let window_id = self.next_window_id.fetch_add(1, Ordering::Relaxed) + 1;
        let window = self.backend.create_window(marker, window_id);
        let window_ref = window.downgrade();
        if let Ok(mut windows) = self.windows.write() {
            windows.insert(window_id, window);
        }
        (window_id, window_ref)
    }

    pub fn window(&self, window_id: WindowId) -> Option<MainActorRef<Window>> {
        self.windows
            .read()
            .ok()
            .and_then(|windows| windows.get(&window_id).map(MainActor::downgrade))
    }

    pub(crate) fn remove_window(&self, window_id: WindowId) {
        if let Ok(mut windows) = self.windows.write() {
            windows.remove(&window_id);
        }
    }
}

pub(crate) trait WindowManagerBackend: Send + Sync {
    fn create_window(&self, marker: MainMarker, window_id: WindowId) -> MainActor<Window>;
}

#[derive(Clone, Debug)]
pub enum WindowError {
    Closed,
    Backend(String),
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WindowError::Closed => write!(f, "window is closed"),
            WindowError::Backend(message) => write!(f, "{message}"),
        }
    }
}

impl std::error::Error for WindowError {}

/// Native window owner.
///
/// A `Window` has the same lifetime as its backend window. Dropping it releases
/// the backend-owned native window handle. External users should hold
/// `MainActorRef<Window>` weak references rather than owning `Window` directly.
pub struct Window {
    backend: Box<dyn WindowBackend>,
}

impl Window {
    pub(crate) fn new(backend: Box<dyn WindowBackend>) -> Self {
        Window { backend }
    }

    pub fn show(&self) -> WindowResult<()> {
        self.backend.show()
    }

    pub fn set_bounds(&self, bounds: Rect) -> WindowResult<()> {
        self.backend.set_bounds(bounds)
    }

    pub fn set_size(&self, width: f64, height: f64) -> WindowResult<()> {
        self.backend.set_size(width, height)
    }

    pub fn set_position(&self, x: f64, y: f64) -> WindowResult<()> {
        self.backend.set_position(x, y)
    }

    pub fn set_commands(&self, commands: WindowCommandSet) -> WindowResult<()> {
        self.backend.set_commands(commands)
    }

    #[cfg(feature = "wgpu")]
    pub fn create_wgpu_surface(
        &self,
        marker: MainMarker,
        rect: Rect,
    ) -> WindowResult<MainActor<WgpuSurface>> {
        self.backend
            .create_wgpu_surface(marker, rect)
            .map(|surface| MainActor::new(marker, surface))
    }
}

pub(crate) trait WindowBackend: Send + Sync {
    fn show(&self) -> WindowResult<()>;
    fn set_bounds(&self, bounds: Rect) -> WindowResult<()>;
    fn set_size(&self, width: f64, height: f64) -> WindowResult<()>;
    fn set_position(&self, x: f64, y: f64) -> WindowResult<()>;
    fn set_commands(&self, commands: WindowCommandSet) -> WindowResult<()>;

    #[cfg(feature = "wgpu")]
    fn create_wgpu_surface(&self, marker: MainMarker, rect: Rect) -> WindowResult<WgpuSurface>;
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
pub enum WindowEventKind {
    Created,
    WillClose,
    /// NOTE: Don't fire [[WindowEventKind::DidClose]] after free a window
    DidClose,
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
    WindowEvent {
        window_id: WindowId,
        kind: WindowEventKind,
    },
    WindowCommand {
        window_id: WindowId,
        command_id: String,
    },
}

pub trait MessageHandler<M: Send + Sync>: Send + Sync {
    fn on_init(&mut self, _marker: MainMarker, _cx: impl Context<M>) {}
    fn on_event(&mut self, _marker: MainMarker, _cx: impl Context<M>, _event: Event<M>) {}
    fn will_terminate(&mut self, _marker: MainMarker, _cx: impl Context<M>) {}
}
