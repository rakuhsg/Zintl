use std::sync::{Arc, OnceLock, RwLock};

use crossbeam::queue::SegQueue;
use zpd_appkit::geometry::Rect as AppkitRect;
use zpd_appkit::runloop::{
    Application as AppkitApplication, ApplicationDelegate as AppkitApplicationDelegate,
    RunLoopScheduler,
};
#[cfg(feature = "wgpu")]
use zpd_appkit::ui::WgpuSurface as NativeAppkitWgpuSurface;
use zpd_appkit::ui::{
    CommandItem as AppkitCommandItem, CommandMenu as AppkitCommandMenu,
    CommandModifier as AppkitCommandModifier, CommandRole as AppkitCommandRole,
    CommandSet as AppkitCommandSet, Window as NativeAppkitWindow,
    WindowAppMenu as AppkitWindowAppMenu, WindowDelegate as NativeAppkitWindowDelegate,
    WindowError as AppkitWindowError,
};

use crate::actor::*;
#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;
use crate::messageloop::{
    Context, Event, MainTask, MessageHandler, Window, WindowAppMenu, WindowBackend,
    WindowCommandItem, WindowCommandMenu, WindowCommandModifier, WindowCommandRole,
    WindowCommandSet, WindowError, WindowEventKind, WindowId, WindowManager, WindowManagerBackend,
    WindowResult,
};
#[cfg(feature = "wgpu")]
use crate::messageloop::{WgpuSurface, WgpuSurfaceBackend};

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
        f: impl FnOnce(MainMarker, Self) + Send + 'static,
        send_after: Option<M>,
    ) {
        self.mesloop.queue.push(MainTask {
            f: Box::new(f),
            send_after,
        });
        self.mesloop.schedule();
    }

    fn send_message(&self, message: M) {
        self.mesloop.queue.push(MainTask {
            f: Box::new(|_, _| {}),
            send_after: Some(message),
        });
        self.mesloop.schedule();
    }

    fn window_manager(&self) -> WindowManager {
        self.mesloop.window_manager.clone()
    }
}

struct AppkitWindowManagerBackend<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
    application: OnceLock<&'static AppkitApplication<AppkitDelegate<M, H>>>,
    window_events: Arc<SegQueue<(WindowId, WindowEventKind)>>,
    command_events: Arc<SegQueue<(WindowId, String)>>,
}

impl<M, H> AppkitWindowManagerBackend<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
    fn new(
        window_events: Arc<SegQueue<(WindowId, WindowEventKind)>>,
        command_events: Arc<SegQueue<(WindowId, String)>>,
    ) -> Self {
        Self {
            application: OnceLock::new(),
            window_events,
            command_events,
        }
    }
}

// SAFETY: The stored AppKit application is process-global and never moved or
// dropped. `create_window` requires a non-Send `MainMarker`, so the application
// reference is only accessed on the AppKit main thread.
unsafe impl<M, H> Send for AppkitWindowManagerBackend<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
}

// SAFETY: See the `Send` implementation. Shared access from other threads only
// reaches the thread-safe event queues.
unsafe impl<M, H> Sync for AppkitWindowManagerBackend<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
}

impl<M, H> WindowManagerBackend for AppkitWindowManagerBackend<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
    fn create_window(&self, marker: MainMarker, window_id: WindowId) -> MainActor<Window> {
        let application: &'static AppkitApplication<AppkitDelegate<M, H>> = self
            .application
            .get()
            .copied()
            .expect("AppKit application must be initialized before creating windows");
        let scheduler = application.scheduler();
        let native = application
            .create_window(AppkitWindowDelegate {
                window_id,
                window_events: self.window_events.clone(),
                scheduler: scheduler.clone(),
            })
            .expect("AppKit failed to create a native window");
        let backend = AppkitWindowBackend { native };

        MainActor::new(marker, Window::new(Box::new(backend)))
    }

    fn set_commands(&self, _marker: MainMarker, commands: WindowCommandSet) -> WindowResult<()> {
        let application = self
            .application
            .get()
            .copied()
            .expect("AppKit application must be initialized before setting commands");
        let command_events = self.command_events.clone();
        let scheduler = application.scheduler();
        application
            .set_commands(&appkit_command_set(commands), move |command_id| {
                command_events.push((0, command_id.to_owned()));
                scheduler.schedule();
            })
            .map_err(|error| WindowError::Backend(error.to_string()))
    }
}

struct AppkitWindowDelegate {
    window_id: WindowId,
    window_events: Arc<SegQueue<(WindowId, WindowEventKind)>>,
    scheduler: RunLoopScheduler,
}

impl NativeAppkitWindowDelegate for AppkitWindowDelegate {
    fn did_create(&mut self) {
        self.window_events
            .push((self.window_id, WindowEventKind::Created));
        self.scheduler.schedule();
    }

    fn will_close(&mut self) {
        self.window_events
            .push((self.window_id, WindowEventKind::WillClose));
        self.scheduler.schedule();
    }

    fn did_close(&mut self) {
        self.window_events
            .push((self.window_id, WindowEventKind::DidClose));
        self.scheduler.schedule();
    }

    fn did_click(&mut self) {
        self.window_events
            .push((self.window_id, WindowEventKind::Click));
        self.scheduler.schedule();
    }
}

struct AppkitWindowBackend {
    native: NativeAppkitWindow<'static, AppkitWindowDelegate>,
}

// SAFETY: `AppkitWindowBackend` is only read through `MainActor` while holding
// a non-Send `MainMarker`. The message loop is process-global, so final native
// destruction also occurs while dispatching on the AppKit main thread.
unsafe impl Send for AppkitWindowBackend {}
// SAFETY: See the `Send` implementation.
unsafe impl Sync for AppkitWindowBackend {}

impl WindowBackend for AppkitWindowBackend {
    fn show(&self) -> WindowResult<()> {
        self.native.show().map_err(window_error)
    }

    fn set_bounds(&self, bounds: Rect) -> WindowResult<()> {
        self.native
            .set_bounds(appkit_rect(bounds))
            .map_err(window_error)
    }

    fn set_size(&self, width: f64, height: f64) -> WindowResult<()> {
        self.native.set_size(width, height).map_err(window_error)
    }

    fn set_position(&self, x: f64, y: f64) -> WindowResult<()> {
        self.native.set_position(x, y).map_err(window_error)
    }

    #[cfg(feature = "wgpu")]
    fn create_wgpu_surface(&self, _marker: MainMarker, rect: Rect) -> WindowResult<WgpuSurface> {
        let native = self
            .native
            .create_wgpu_surface(appkit_rect(rect))
            .map_err(window_error)?;
        Ok(WgpuSurface::new(Box::new(AppkitWgpuSurfaceBackend {
            native,
        })))
    }
}

#[cfg(feature = "wgpu")]
struct AppkitWgpuSurfaceBackend {
    native: NativeAppkitWgpuSurface<'static>,
}

// SAFETY: This backend is only accessed through a main-actor `WgpuSurface`.
// Its process-global application lifetime is encoded as `'static`.
#[cfg(feature = "wgpu")]
unsafe impl Send for AppkitWgpuSurfaceBackend {}
// SAFETY: See the `Send` implementation.
#[cfg(feature = "wgpu")]
unsafe impl Sync for AppkitWgpuSurfaceBackend {}

#[cfg(feature = "wgpu")]
impl WgpuSurfaceBackend for AppkitWgpuSurfaceBackend {
    fn surface_target_unsafe(&self) -> wgpu::SurfaceTargetUnsafe {
        let layer = self
            .native
            .metal_layer()
            .expect("AppKit returned a null CAMetalLayer");
        wgpu::SurfaceTargetUnsafe::CoreAnimationLayer(layer.as_ptr())
    }

    fn drawable_size(&self) -> PhysicalSize {
        let size = self.native.drawable_size();
        PhysicalSize {
            width: size.width,
            height: size.height,
        }
    }

    fn set_rect(&self, rect: Rect) {
        self.native.set_rect(appkit_rect(rect));
    }
}

struct AppkitDelegate<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
    mesloop: Arc<AppkitMessageLoop<M, H>>,
}

impl<M, H> AppkitApplicationDelegate for AppkitDelegate<M, H>
where
    M: Send + Sync + 'static,
    H: MessageHandler<M> + 'static,
{
    fn on_launch(&mut self) {
        let cx = self.mesloop.clone().context();
        let mut handler = self.mesloop.handler.write().unwrap();
        handler.on_init(MainMarker::new(), cx);
    }

    fn perform(&mut self) {
        while let Some(task) = self.mesloop.queue.pop() {
            let cx = self.mesloop.clone().context();
            (task.f)(MainMarker::new(), cx);

            if let Some(message) = task.send_after {
                self.mesloop.dispatch_message(message);
            }

            self.mesloop.dispatch_pending_window_events();
            self.mesloop.dispatch_pending_command_events();
        }

        self.mesloop.dispatch_pending_window_events();
        self.mesloop.dispatch_pending_command_events();
    }

    fn will_terminate(&mut self) {
        let cx = self.mesloop.clone().context();
        let mut handler = self.mesloop.handler.write().unwrap();
        handler.will_terminate(MainMarker::new(), cx);
    }
}

pub struct AppkitMessageLoop<M: Send + Sync, H: MessageHandler<M>> {
    handler: RwLock<H>,
    queue: SegQueue<MainTask<AppkitContext<M, H>, M>>,
    scheduler: OnceLock<RunLoopScheduler>,
    window_events: Arc<SegQueue<(WindowId, WindowEventKind)>>,
    command_events: Arc<SegQueue<(WindowId, String)>>,
    window_manager: WindowManager,
}

impl<M: Send + Sync + 'static, H: MessageHandler<M> + 'static> AppkitMessageLoop<M, H> {
    pub fn new(handler: H) -> Arc<Self> {
        let window_events = Arc::new(SegQueue::new());
        let command_events = Arc::new(SegQueue::new());
        let window_backend = Arc::new(AppkitWindowManagerBackend::new(
            window_events.clone(),
            command_events.clone(),
        ));
        let window_manager = WindowManager::new(window_backend.clone());
        let mesloop = Arc::new(Self {
            handler: handler.into(),
            queue: SegQueue::new(),
            scheduler: OnceLock::new(),
            window_events,
            command_events,
            window_manager,
        });
        let application = AppkitApplication::new(AppkitDelegate {
            mesloop: mesloop.clone(),
        })
        .expect("AppKit message loop must be initialized once on the main thread");

        // AppKit is process-global and runs until process termination. Leaking
        // this owner also keeps the message loop and all native lifetimes valid.
        let application: &'static AppkitApplication<AppkitDelegate<M, H>> =
            Box::leak(Box::new(application));
        window_backend
            .application
            .set(application)
            .unwrap_or_else(|_| panic!("AppKit application was initialized twice"));
        mesloop
            .scheduler
            .set(application.scheduler())
            .unwrap_or_else(|_| panic!("AppKit scheduler was initialized twice"));

        mesloop
    }

    fn scheduler(&self) -> &RunLoopScheduler {
        self.scheduler
            .get()
            .expect("AppKit scheduler must be initialized")
    }

    fn schedule(&self) {
        self.scheduler().schedule();
    }

    fn context(self: Arc<Self>) -> AppkitContext<M, H> {
        AppkitContext::new(self)
    }

    fn dispatch_message(self: &Arc<Self>, message: M) {
        let cx = self.clone().context();
        let mut handler = self.handler.write().unwrap();
        handler.on_event(MainMarker::new(), cx, Event::UserMessage(message));
    }

    fn dispatch_window_event(self: &Arc<Self>, window_id: WindowId, kind: WindowEventKind) {
        if matches!(kind, WindowEventKind::DidClose) {
            self.window_manager.remove_window(window_id);
        }

        let cx = self.clone().context();
        let mut handler = self.handler.write().unwrap();
        handler.on_event(
            MainMarker::new(),
            cx,
            Event::WindowEvent {
                window_id,
                kind: kind.clone(),
            },
        );
    }

    fn dispatch_pending_window_events(self: &Arc<Self>) {
        while let Some((window_id, kind)) = self.window_events.pop() {
            self.dispatch_window_event(window_id, kind);
        }
    }

    fn dispatch_command_event(self: &Arc<Self>, window_id: WindowId, command_id: String) {
        let cx = self.clone().context();
        let mut handler = self.handler.write().unwrap();
        handler.on_event(
            MainMarker::new(),
            cx,
            Event::WindowCommand {
                window_id,
                command_id,
            },
        );
    }

    fn dispatch_pending_command_events(self: &Arc<Self>) {
        while let Some((window_id, command_id)) = self.command_events.pop() {
            self.dispatch_command_event(window_id, command_id);
        }
    }

    pub fn run(&self) {
        self.scheduler()
            .run()
            .expect("AppKit run loop must run on the main thread");
    }
}

fn appkit_rect(rect: Rect) -> AppkitRect {
    AppkitRect::new(rect.x, rect.y, rect.width, rect.height)
}

fn window_error(error: AppkitWindowError) -> WindowError {
    match error {
        AppkitWindowError::Closed => WindowError::Closed,
        error => WindowError::Backend(error.to_string()),
    }
}

fn appkit_command_set(commands: WindowCommandSet) -> AppkitCommandSet {
    AppkitCommandSet {
        app_menu: commands.app_menu.map(appkit_app_menu),
        menus: commands
            .menus
            .into_iter()
            .map(appkit_command_menu)
            .collect(),
    }
}

fn appkit_app_menu(menu: WindowAppMenu) -> AppkitWindowAppMenu {
    AppkitWindowAppMenu {
        items: menu.items.into_iter().map(appkit_command_item).collect(),
    }
}

fn appkit_command_menu(menu: WindowCommandMenu) -> AppkitCommandMenu {
    AppkitCommandMenu {
        title: menu.title,
        items: menu.items.into_iter().map(appkit_command_item).collect(),
    }
}

fn appkit_command_item(item: WindowCommandItem) -> AppkitCommandItem {
    AppkitCommandItem {
        id: item.id,
        title: item.title,
        role: item.role.map(appkit_command_role),
        key: item.key,
        modifiers: item
            .modifiers
            .into_iter()
            .map(appkit_command_modifier)
            .collect(),
        enabled: item.enabled,
    }
}

fn appkit_command_modifier(modifier: WindowCommandModifier) -> AppkitCommandModifier {
    match modifier {
        WindowCommandModifier::Cmd => AppkitCommandModifier::Cmd,
        WindowCommandModifier::Ctrl => AppkitCommandModifier::Ctrl,
        WindowCommandModifier::Alt => AppkitCommandModifier::Alt,
        WindowCommandModifier::Shift => AppkitCommandModifier::Shift,
    }
}

fn appkit_command_role(role: WindowCommandRole) -> AppkitCommandRole {
    match role {
        WindowCommandRole::About => AppkitCommandRole::About,
        WindowCommandRole::Quit => AppkitCommandRole::Quit,
    }
}
