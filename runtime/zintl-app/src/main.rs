use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};
use std::thread;

use futures::channel::oneshot;
use zintl_deno::api::{
    ZintlApi, ZintlAppApi, ZintlAppError, ZintlAppEvent, ZintlWindowApi, ZintlWindowAppMenu,
    ZintlWindowBounds, ZintlWindowCommandItem, ZintlWindowCommandMenu, ZintlWindowCommandModifier,
    ZintlWindowCommandRole, ZintlWindowCommandSet, ZintlWindowCreateOptions, ZintlWindowError,
    ZintlWindowFuture, ZintlWindowId, ZintlWindowPosition, ZintlWindowSize,
};
use zintl_deno::runtime::{DenoRuntime, DenoRuntimeOptions};
use zintl_native::{
    Context, Event, MainActorError, MainActorRef, MainMarker, MessageHandler, PlatformMessageLoop,
    Rect, Window, WindowAppMenu as NativeWindowAppMenu, WindowCommandEvent,
    WindowCommandItem as NativeWindowCommandItem, WindowCommandMenu as NativeWindowCommandMenu,
    WindowCommandModifier as NativeWindowCommandModifier,
    WindowCommandRole as NativeWindowCommandRole, WindowCommandSet as NativeWindowCommandSet,
    WindowError, WindowEvent, WindowManager,
};

struct Handler {
    main_module: PathBuf,
    js_thread: Option<thread::JoinHandle<()>>,
    window_state: Arc<AppWindowState>,
}

impl Handler {
    fn new(main_module: PathBuf) -> Self {
        Handler {
            main_module,
            js_thread: None,
            window_state: Arc::new(AppWindowState::default()),
        }
    }

    fn start_js_thread(&mut self, options: DenoRuntimeOptions) {
        let main_module = self.main_module.clone();
        self.js_thread = Some(
            thread::Builder::new()
                .name("zintl-js".to_string())
                .spawn(move || {
                    if let Err(error) =
                        DenoRuntime::run_file_path_current_thread_with_options(main_module, options)
                    {
                        eprintln!("zintl-js: {error}");
                    }
                })
                .expect("failed to spawn JS thread"),
        );
    }
}

impl MessageHandler<Message> for Handler {
    fn on_init(&mut self, _marker: MainMarker, cx: impl Context<Message>) {
        self.window_state.set_window_manager(cx.window_manager());
        let host = Arc::new(AppWindowHost {
            cx,
            state: self.window_state.clone(),
        });
        let app: Arc<dyn ZintlAppApi> = host.clone();
        let window: Arc<dyn ZintlWindowApi> = host;
        self.start_js_thread(DenoRuntimeOptions {
            api: ZintlApi {
                app: Some(app),
                window: Some(window),
            },
        });
    }

    fn on_event(&mut self, _marker: MainMarker, _cx: impl Context<Message>, event: Event<Message>) {
        match event {
            Event::UserMessage(Message::WindowOperationCompleted {
                operation_id,
                result,
            }) => {
                self.window_state
                    .complete_window_operation(operation_id, result);
            }
            Event::WindowEvent { window_id, event } => match event {
                WindowEvent::Created => self
                    .window_state
                    .push_app_event(ZintlAppEvent::WindowCreated { window_id }),
                WindowEvent::WillClose => self
                    .window_state
                    .push_app_event(ZintlAppEvent::WindowWillClose { window_id }),
                WindowEvent::DidClose => {}
            },
        }
    }
}

enum Message {
    WindowOperationCompleted {
        operation_id: WindowOperationId,
        result: WindowOperationResult,
    },
}

type WindowOperationId = u32;

enum PendingWindowOperation {
    Create(oneshot::Sender<Result<ZintlWindowId, ZintlWindowError>>),
    Unit(oneshot::Sender<Result<(), ZintlWindowError>>),
}

enum WindowOperationResult {
    Create(Result<ZintlWindowId, ZintlWindowError>),
    Unit(Result<(), ZintlWindowError>),
}

#[derive(Default)]
struct AppWindowState {
    next_operation_id: AtomicU32,
    window_manager: RwLock<Option<WindowManager>>,
    app_events: RwLock<VecDeque<ZintlAppEvent>>,
    pending_window_operations: RwLock<HashMap<WindowOperationId, PendingWindowOperation>>,
}

struct AppWindowHost<C> {
    cx: C,
    state: Arc<AppWindowState>,
}

impl<C> ZintlWindowApi for AppWindowHost<C>
where
    C: Context<Message> + Send + Sync,
{
    fn create_window(&self, options: ZintlWindowCreateOptions) -> ZintlWindowFuture<ZintlWindowId> {
        let (operation_id, receiver) = match self.state.begin_create_window_operation() {
            Ok(operation) => operation,
            Err(error) => return failed_window_future(error),
        };
        if let Err(error) = validate_create_options(&options) {
            self.cx.send_message(Message::WindowOperationCompleted {
                operation_id,
                result: WindowOperationResult::Create(Err(error)),
            });
            return create_window_future(receiver);
        }

        let state = self.state.clone();
        self.cx.perform_main(
            move |marker, cx| {
                let wm = match state.window_manager() {
                    Ok(wm) => wm,
                    Err(error) => {
                        cx.send_message(Message::WindowOperationCompleted {
                            operation_id,
                            result: WindowOperationResult::Create(Err(error)),
                        });
                        return;
                    }
                };
                let (window_id, window) = wm.create_window(marker);
                let result = window
                    .upgrade()
                    .map_err(|error| window_actor_error(window_id, error))
                    .and_then(|window| {
                        let native_window = window
                            .read(marker)
                            .map_err(|error| window_actor_error(window_id, error))?;
                        apply_create_options(window_id, &state, &native_window, options)?;
                        native_window
                            .show()
                            .map_err(|error| window_backend_error(window_id, error))
                    })
                    .map(|()| window_id);

                cx.send_message(Message::WindowOperationCompleted {
                    operation_id,
                    result: WindowOperationResult::Create(result),
                });
            },
            None,
        );
        create_window_future(receiver)
    }

    fn set_window_bounds(
        &self,
        window_id: ZintlWindowId,
        bounds: ZintlWindowBounds,
    ) -> ZintlWindowFuture<()> {
        let (operation_id, receiver) = match self.state.begin_unit_window_operation() {
            Ok(operation) => operation,
            Err(error) => return failed_window_future(error),
        };
        if let Err(error) = validate_bounds(bounds) {
            self.cx.send_message(Message::WindowOperationCompleted {
                operation_id,
                result: WindowOperationResult::Unit(Err(error)),
            });
            return unit_window_future(receiver);
        }

        let state = self.state.clone();
        self.cx.perform_main(
            move |marker, cx| {
                let result = state.with_window(window_id, marker, |window| {
                    window
                        .set_bounds(rect_from_bounds(bounds))
                        .map_err(|error| window_backend_error(window_id, error))
                });
                cx.send_message(Message::WindowOperationCompleted {
                    operation_id,
                    result: WindowOperationResult::Unit(result),
                });
            },
            None,
        );
        unit_window_future(receiver)
    }

    fn set_window_size(
        &self,
        window_id: ZintlWindowId,
        size: ZintlWindowSize,
    ) -> ZintlWindowFuture<()> {
        let (operation_id, receiver) = match self.state.begin_unit_window_operation() {
            Ok(operation) => operation,
            Err(error) => return failed_window_future(error),
        };
        if let Err(error) = validate_size(size) {
            self.cx.send_message(Message::WindowOperationCompleted {
                operation_id,
                result: WindowOperationResult::Unit(Err(error)),
            });
            return unit_window_future(receiver);
        }

        let state = self.state.clone();
        self.cx.perform_main(
            move |marker, cx| {
                let result = state.with_window(window_id, marker, |window| {
                    window
                        .set_size(size.width, size.height)
                        .map_err(|error| window_backend_error(window_id, error))
                });
                cx.send_message(Message::WindowOperationCompleted {
                    operation_id,
                    result: WindowOperationResult::Unit(result),
                });
            },
            None,
        );
        unit_window_future(receiver)
    }

    fn set_window_position(
        &self,
        window_id: ZintlWindowId,
        position: ZintlWindowPosition,
    ) -> ZintlWindowFuture<()> {
        let (operation_id, receiver) = match self.state.begin_unit_window_operation() {
            Ok(operation) => operation,
            Err(error) => return failed_window_future(error),
        };
        if let Err(error) = validate_position(position) {
            self.cx.send_message(Message::WindowOperationCompleted {
                operation_id,
                result: WindowOperationResult::Unit(Err(error)),
            });
            return unit_window_future(receiver);
        }

        let state = self.state.clone();
        self.cx.perform_main(
            move |marker, cx| {
                let result = state.with_window(window_id, marker, |window| {
                    window
                        .set_position(position.x, position.y)
                        .map_err(|error| window_backend_error(window_id, error))
                });
                cx.send_message(Message::WindowOperationCompleted {
                    operation_id,
                    result: WindowOperationResult::Unit(result),
                });
            },
            None,
        );
        unit_window_future(receiver)
    }

    fn set_window_commands(
        &self,
        window_id: ZintlWindowId,
        commands: ZintlWindowCommandSet,
    ) -> ZintlWindowFuture<()> {
        let (operation_id, receiver) = match self.state.begin_unit_window_operation() {
            Ok(operation) => operation,
            Err(error) => return failed_window_future(error),
        };
        if let Err(error) = validate_commands(&commands) {
            self.cx.send_message(Message::WindowOperationCompleted {
                operation_id,
                result: WindowOperationResult::Unit(Err(error)),
            });
            return unit_window_future(receiver);
        }

        let state = self.state.clone();
        self.cx.perform_main(
            move |marker, cx| {
                let result = state.with_window(window_id, marker, |window| {
                    set_native_commands(window_id, state.clone(), window, commands)
                });
                cx.send_message(Message::WindowOperationCompleted {
                    operation_id,
                    result: WindowOperationResult::Unit(result),
                });
            },
            None,
        );
        unit_window_future(receiver)
    }
}

impl<C> ZintlAppApi for AppWindowHost<C>
where
    C: Context<Message> + Send + Sync,
{
    fn take_event(&self) -> Result<Option<ZintlAppEvent>, ZintlAppError> {
        let mut app_events = self
            .state
            .app_events
            .write()
            .map_err(|_| ZintlAppError::new("app event queue is poisoned"))?;
        Ok(app_events.pop_front())
    }
}

impl AppWindowState {
    fn set_window_manager(&self, window_manager: WindowManager) {
        if let Ok(mut current) = self.window_manager.write() {
            if current.is_none() {
                *current = Some(window_manager);
            }
        }
    }

    fn window_manager(&self) -> Result<WindowManager, ZintlWindowError> {
        self.window_manager
            .read()
            .map_err(|_| ZintlWindowError::new("window manager registry is poisoned"))?
            .clone()
            .ok_or_else(|| ZintlWindowError::new("window manager is not initialized"))
    }

    fn begin_create_window_operation(
        &self,
    ) -> Result<
        (
            WindowOperationId,
            oneshot::Receiver<Result<ZintlWindowId, ZintlWindowError>>,
        ),
        ZintlWindowError,
    > {
        let operation_id = self.next_operation_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = oneshot::channel();
        let mut operations = self
            .pending_window_operations
            .write()
            .map_err(|_| ZintlWindowError::new("window operation registry is poisoned"))?;
        operations.insert(operation_id, PendingWindowOperation::Create(sender));
        Ok((operation_id, receiver))
    }

    fn begin_unit_window_operation(
        &self,
    ) -> Result<
        (
            WindowOperationId,
            oneshot::Receiver<Result<(), ZintlWindowError>>,
        ),
        ZintlWindowError,
    > {
        let operation_id = self.next_operation_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (sender, receiver) = oneshot::channel();
        let mut operations = self
            .pending_window_operations
            .write()
            .map_err(|_| ZintlWindowError::new("window operation registry is poisoned"))?;
        operations.insert(operation_id, PendingWindowOperation::Unit(sender));
        Ok((operation_id, receiver))
    }

    fn complete_window_operation(
        &self,
        operation_id: WindowOperationId,
        result: WindowOperationResult,
    ) {
        let operation = self
            .pending_window_operations
            .write()
            .ok()
            .and_then(|mut operations| operations.remove(&operation_id));

        match (operation, result) {
            (
                Some(PendingWindowOperation::Create(sender)),
                WindowOperationResult::Create(result),
            ) => {
                let _ = sender.send(result);
            }
            (Some(PendingWindowOperation::Unit(sender)), WindowOperationResult::Unit(result)) => {
                let _ = sender.send(result);
            }
            _ => {}
        }
    }

    fn window(&self, window_id: ZintlWindowId) -> Option<MainActorRef<Window>> {
        self.window_manager
            .read()
            .ok()
            .and_then(|window_manager| window_manager.as_ref().and_then(|wm| wm.window(window_id)))
    }

    fn with_window<T>(
        &self,
        window_id: ZintlWindowId,
        marker: MainMarker,
        f: impl FnOnce(&Window) -> Result<T, ZintlWindowError>,
    ) -> Result<T, ZintlWindowError> {
        let window = self
            .window(window_id)
            .ok_or_else(|| window_not_found_error(window_id))?
            .upgrade()
            .map_err(|error| window_actor_error(window_id, error))?;
        let window = window
            .read(marker)
            .map_err(|error| window_actor_error(window_id, error))?;
        f(&window)
    }

    fn push_app_event(&self, event: ZintlAppEvent) {
        if let Ok(mut events) = self.app_events.write() {
            events.push_back(event);
        }
    }

    fn push_command_event(&self, window_id: ZintlWindowId, event: WindowCommandEvent) {
        if let Ok(mut events) = self.app_events.write() {
            events.push_back(ZintlAppEvent::WindowCommand {
                window_id,
                command_id: event.command_id,
            });
        }
    }
}

fn failed_window_future<T: Send + 'static>(error: ZintlWindowError) -> ZintlWindowFuture<T> {
    Box::pin(async move { Err(error) })
}

fn create_window_future(
    receiver: oneshot::Receiver<Result<ZintlWindowId, ZintlWindowError>>,
) -> ZintlWindowFuture<ZintlWindowId> {
    Box::pin(async move {
        receiver.await.unwrap_or_else(|_| {
            Err(ZintlWindowError::new(
                "window create operation was canceled",
            ))
        })
    })
}

fn unit_window_future(
    receiver: oneshot::Receiver<Result<(), ZintlWindowError>>,
) -> ZintlWindowFuture<()> {
    Box::pin(async move {
        receiver.await.unwrap_or_else(|_| {
            Err(ZintlWindowError::new(
                "window operation completion was canceled",
            ))
        })
    })
}

fn window_not_found_error(window_id: ZintlWindowId) -> ZintlWindowError {
    ZintlWindowError::new(format!("window {window_id} does not exist"))
}

fn window_actor_error(window_id: ZintlWindowId, error: MainActorError) -> ZintlWindowError {
    match error {
        MainActorError::Dropped => window_not_found_error(window_id),
        MainActorError::LockError => ZintlWindowError::new("window registry is poisoned"),
        MainActorError::NotInMainThread => {
            ZintlWindowError::new("window operation must run on the main thread")
        }
    }
}

fn window_backend_error(window_id: ZintlWindowId, error: WindowError) -> ZintlWindowError {
    match error {
        WindowError::Closed => window_not_found_error(window_id),
        WindowError::Backend(message) => ZintlWindowError::new(message),
    }
}

fn apply_create_options(
    window_id: ZintlWindowId,
    state: &Arc<AppWindowState>,
    window: &Window,
    options: ZintlWindowCreateOptions,
) -> Result<(), ZintlWindowError> {
    if let Some(bounds) = options.bounds {
        window
            .set_bounds(rect_from_bounds(bounds))
            .map_err(|error| window_backend_error(window_id, error))?;
    } else {
        if let Some(size) = options.size {
            window
                .set_size(size.width, size.height)
                .map_err(|error| window_backend_error(window_id, error))?;
        }
        if let Some(position) = options.position {
            window
                .set_position(position.x, position.y)
                .map_err(|error| window_backend_error(window_id, error))?;
        }
    }

    if let Some(commands) = options.commands {
        set_native_commands(window_id, state.clone(), window, commands)?;
    }
    Ok(())
}

fn rect_from_bounds(bounds: ZintlWindowBounds) -> Rect {
    Rect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

fn set_native_commands(
    window_id: ZintlWindowId,
    state: Arc<AppWindowState>,
    window: &Window,
    commands: ZintlWindowCommandSet,
) -> Result<(), ZintlWindowError> {
    window
        .set_commands(
            native_command_set(commands),
            Arc::new(move |event: WindowCommandEvent| {
                state.push_command_event(window_id, event);
            }),
        )
        .map_err(|error| window_backend_error(window_id, error))
}

fn native_command_set(commands: ZintlWindowCommandSet) -> NativeWindowCommandSet {
    NativeWindowCommandSet {
        app_menu: commands.app_menu.map(native_app_menu),
        menus: commands
            .menus
            .into_iter()
            .map(native_command_menu)
            .collect(),
    }
}

fn native_app_menu(menu: ZintlWindowAppMenu) -> NativeWindowAppMenu {
    NativeWindowAppMenu {
        items: menu.items.into_iter().map(native_command_item).collect(),
    }
}

fn native_command_menu(menu: ZintlWindowCommandMenu) -> NativeWindowCommandMenu {
    NativeWindowCommandMenu {
        title: menu.title,
        items: menu.items.into_iter().map(native_command_item).collect(),
    }
}

fn native_command_item(item: ZintlWindowCommandItem) -> NativeWindowCommandItem {
    NativeWindowCommandItem {
        id: item.id,
        title: item.title,
        role: item.role.map(native_role),
        key: item.key,
        modifiers: item.modifiers.into_iter().map(native_modifier).collect(),
        enabled: item.enabled,
    }
}

fn native_modifier(modifier: ZintlWindowCommandModifier) -> NativeWindowCommandModifier {
    match modifier {
        ZintlWindowCommandModifier::Cmd => NativeWindowCommandModifier::Cmd,
        ZintlWindowCommandModifier::Ctrl => NativeWindowCommandModifier::Ctrl,
        ZintlWindowCommandModifier::Alt => NativeWindowCommandModifier::Alt,
        ZintlWindowCommandModifier::Shift => NativeWindowCommandModifier::Shift,
    }
}

fn native_role(role: ZintlWindowCommandRole) -> NativeWindowCommandRole {
    match role {
        ZintlWindowCommandRole::About => NativeWindowCommandRole::About,
        ZintlWindowCommandRole::Quit => NativeWindowCommandRole::Quit,
    }
}

fn validate_create_options(options: &ZintlWindowCreateOptions) -> Result<(), ZintlWindowError> {
    if let Some(bounds) = options.bounds {
        validate_bounds(bounds)?;
    }
    if let Some(size) = options.size {
        validate_size(size)?;
    }
    if let Some(position) = options.position {
        validate_position(position)?;
    }
    if let Some(commands) = options.commands.as_ref() {
        validate_commands(commands)?;
    }
    Ok(())
}

fn validate_bounds(bounds: ZintlWindowBounds) -> Result<(), ZintlWindowError> {
    validate_position(ZintlWindowPosition {
        x: bounds.x,
        y: bounds.y,
    })?;
    validate_size(ZintlWindowSize {
        width: bounds.width,
        height: bounds.height,
    })
}

fn validate_size(size: ZintlWindowSize) -> Result<(), ZintlWindowError> {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return Err(ZintlWindowError::new(
            "window size must contain positive finite width and height",
        ));
    }
    Ok(())
}

fn validate_position(position: ZintlWindowPosition) -> Result<(), ZintlWindowError> {
    if !position.x.is_finite() || !position.y.is_finite() {
        return Err(ZintlWindowError::new(
            "window position must contain finite x and y",
        ));
    }
    Ok(())
}

fn validate_commands(commands: &ZintlWindowCommandSet) -> Result<(), ZintlWindowError> {
    if let Some(app_menu) = commands.app_menu.as_ref() {
        validate_command_items(&app_menu.items)?;
    }
    for menu in &commands.menus {
        if menu.title.is_empty() {
            return Err(ZintlWindowError::new(
                "window command menu title cannot be empty",
            ));
        }
        validate_command_items(&menu.items)?;
    }
    Ok(())
}

fn validate_command_items(items: &[ZintlWindowCommandItem]) -> Result<(), ZintlWindowError> {
    for item in items {
        let has_command_id = item.id.as_ref().is_some_and(|id| !id.is_empty());
        if !has_command_id && item.role.is_none() {
            return Err(ZintlWindowError::new(
                "window command id cannot be empty unless role is set",
            ));
        }
        if has_command_id && item.role.is_some() {
            return Err(ZintlWindowError::new(
                "window command cannot set both id and role",
            ));
        }
        if item.title.is_empty() {
            return Err(ZintlWindowError::new(
                "window command title cannot be empty",
            ));
        }
    }
    Ok(())
}

fn main() {
    let main_module = main_module_from_args();
    let handler = Handler::new(main_module);
    let m = PlatformMessageLoop::new(handler);
    m.run();
}

fn main_module_from_args() -> PathBuf {
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: zintl-app <main.js>");
        std::process::exit(2);
    };

    let path = PathBuf::from(path);
    match path.canonicalize() {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "failed to resolve main module '{}': {error}",
                path.display()
            );
            std::process::exit(2);
        }
    }
}
