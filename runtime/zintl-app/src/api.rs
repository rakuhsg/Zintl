use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, RwLock};

use futures::channel::oneshot;
use zintl_deno::api::{
    ZintlAppApi, ZintlAppError, ZintlAppEvent, ZintlWindowApi, ZintlWindowAppMenu,
    ZintlWindowBounds, ZintlWindowCommandItem, ZintlWindowCommandMenu, ZintlWindowCommandModifier,
    ZintlWindowCommandRole, ZintlWindowCommandSet, ZintlWindowCreateOptions, ZintlWindowError,
    ZintlWindowFuture, ZintlWindowId, ZintlWindowPosition, ZintlWindowSize,
};
use zintl_native::{
    Context, Event, MainActorError, MainActorRef, MainMarker, Rect, Window,
    WindowAppMenu as NativeWindowAppMenu, WindowCommandItem as NativeWindowCommandItem,
    WindowCommandMenu as NativeWindowCommandMenu,
    WindowCommandModifier as NativeWindowCommandModifier,
    WindowCommandRole as NativeWindowCommandRole, WindowCommandSet as NativeWindowCommandSet,
    WindowError, WindowEventKind, WindowManager,
};

pub(crate) enum Message {
    WindowOperationCompleted {
        operation_id: WindowOperationId,
        result: WindowOperationResult,
    },
}

pub(crate) type WindowOperationId = u32;

enum PendingWindowOperation {
    Create(oneshot::Sender<Result<ZintlWindowId, ZintlWindowError>>),
    Unit(oneshot::Sender<Result<(), ZintlWindowError>>),
}

pub(crate) enum WindowOperationResult {
    Create(Result<ZintlWindowId, ZintlWindowError>),
    Unit(Result<(), ZintlWindowError>),
}

#[derive(Default)]
pub(crate) struct AppWindowState {
    next_operation_id: AtomicU32,
    window_manager: RwLock<Option<WindowManager>>,
    app_events: RwLock<VecDeque<ZintlAppEvent>>,
    pending_window_operations: RwLock<HashMap<WindowOperationId, PendingWindowOperation>>,
}

pub(crate) struct AppWindowHost<C> {
    cx: C,
    state: Arc<AppWindowState>,
}

impl<C> AppWindowHost<C>
where
    C: Context<Message> + Send + Sync,
{
    pub(crate) fn new(cx: C, state: Arc<AppWindowState>) -> Self {
        state.set_window_manager(cx.window_manager());
        Self { cx, state }
    }
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
            return ZintlWindowFutureFactory::new(receiver)
                .set_error_message("window create operation was canceled")
                .build();
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
                        apply_create_options(window_id, &native_window, options)?;
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
        ZintlWindowFutureFactory::new(receiver)
            .set_error_message("window create operation was canceled")
            .build()
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
            return ZintlWindowFutureFactory::new(receiver)
                .set_error_message("window operation completion was canceled")
                .build();
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
        ZintlWindowFutureFactory::new(receiver)
            .set_error_message("window operation completion was canceled")
            .build()
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
            return ZintlWindowFutureFactory::new(receiver)
                .set_error_message("window operation completion was canceled")
                .build();
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
        ZintlWindowFutureFactory::new(receiver)
            .set_error_message("window operation completion was canceled")
            .build()
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
            return ZintlWindowFutureFactory::new(receiver)
                .set_error_message("window operation completion was canceled")
                .build();
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
        ZintlWindowFutureFactory::new(receiver)
            .set_error_message("window operation completion was canceled")
            .build()
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
            return ZintlWindowFutureFactory::new(receiver)
                .set_error_message("window operation completion was canceled")
                .build();
        }

        let state = self.state.clone();
        self.cx.perform_main(
            move |marker, cx| {
                let result = state.with_window(window_id, marker, |window| {
                    set_native_commands(window_id, window, commands)
                });
                cx.send_message(Message::WindowOperationCompleted {
                    operation_id,
                    result: WindowOperationResult::Unit(result),
                });
            },
            None,
        );
        ZintlWindowFutureFactory::new(receiver)
            .set_error_message("window operation completion was canceled")
            .build()
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
    pub(crate) fn handle_event(&self, event: Event<Message>) {
        match event {
            Event::UserMessage(Message::WindowOperationCompleted {
                operation_id,
                result,
            }) => {
                self.complete_window_operation(operation_id, result);
            }
            Event::WindowEvent { window_id, kind } => match kind {
                WindowEventKind::Created => {
                    self.push_app_event(ZintlAppEvent::WindowCreated { window_id })
                }
                WindowEventKind::WillClose => {
                    self.push_app_event(ZintlAppEvent::WindowWillClose { window_id })
                }
                WindowEventKind::DidClose => {}
            },
            Event::WindowCommand {
                window_id,
                command_id,
            } => self.push_app_event(ZintlAppEvent::WindowCommand {
                window_id,
                command_id,
            }),
        }
    }

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
}

struct ZintlWindowFutureFactory<T> {
    receiver: oneshot::Receiver<Result<T, ZintlWindowError>>,
    error_message: String,
}

impl<T> ZintlWindowFutureFactory<T>
where
    T: Send + 'static,
{
    fn new(receiver: oneshot::Receiver<Result<T, ZintlWindowError>>) -> Self {
        Self {
            receiver,
            error_message: String::new(),
        }
    }

    fn set_error_message(mut self, error_message: impl Into<String>) -> Self {
        self.error_message = error_message.into();
        self
    }

    fn build(self) -> ZintlWindowFuture<T> {
        let receiver = self.receiver;
        let error_message = self.error_message;
        Box::pin(async move {
            receiver
                .await
                .unwrap_or_else(|_| Err(ZintlWindowError::new(error_message)))
        })
    }
}

fn failed_window_future<T: Send + 'static>(error: ZintlWindowError) -> ZintlWindowFuture<T> {
    Box::pin(async move { Err(error) })
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
        set_native_commands(window_id, window, commands)?;
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
    window: &Window,
    commands: ZintlWindowCommandSet,
) -> Result<(), ZintlWindowError> {
    window
        .set_commands(native_command_set(commands))
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
