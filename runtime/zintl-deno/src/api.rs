use std::cell::RefCell;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;

use deno_error::JsErrorBox;
use deno_runtime::deno_core;
use deno_runtime::deno_core::Extension;
use deno_runtime::deno_core::OpState;
use deno_runtime::deno_core::op2;
use serde::Deserialize;

mod app;

pub use app::ZintlAppEvent;

pub type ZintlWindowFuture<T> = Pin<Box<dyn Future<Output = Result<T, ZintlWindowError>> + Send>>;
pub type ZintlAppEventFuture =
    Pin<Box<dyn Future<Output = Result<ZintlAppEvent, ZintlAppError>> + Send>>;

pub trait ZintlWindowApi: Send + Sync + 'static {
    fn create_window(&self, options: ZintlWindowCreateOptions) -> ZintlWindowFuture<ZintlWindowId>;
    fn set_window_bounds(
        &self,
        window_id: ZintlWindowId,
        bounds: ZintlWindowBounds,
    ) -> ZintlWindowFuture<()>;
    fn set_window_size(
        &self,
        window_id: ZintlWindowId,
        size: ZintlWindowSize,
    ) -> ZintlWindowFuture<()>;
    fn set_window_position(
        &self,
        window_id: ZintlWindowId,
        position: ZintlWindowPosition,
    ) -> ZintlWindowFuture<()>;
}

pub trait ZintlAppApi: Send + Sync + 'static {
    fn next_event(&self) -> ZintlAppEventFuture;
    fn set_commands(&self, commands: ZintlAppCommands) -> Result<(), ZintlAppError>;
}

pub type ZintlWindowId = u32;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZintlWindowCreateOptions {
    pub bounds: Option<ZintlWindowBounds>,
    pub size: Option<ZintlWindowSize>,
    pub position: Option<ZintlWindowPosition>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct ZintlWindowBounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct ZintlWindowSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct ZintlWindowPosition {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub struct ZintlAppCommands {
    #[serde(default, rename = "appMenu")]
    pub app_menu: Option<ZintlAppMenu>,
    #[serde(default)]
    pub menus: Vec<ZintlCommandMenu>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ZintlAppMenu {
    pub items: Vec<ZintlCommandItem>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ZintlCommandMenu {
    pub title: String,
    pub items: Vec<ZintlCommandItem>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ZintlCommandItem {
    pub id: Option<String>,
    pub title: String,
    pub role: Option<ZintlCommandRole>,
    pub key: Option<String>,
    #[serde(default)]
    pub modifiers: Vec<ZintlCommandModifier>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZintlCommandModifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZintlCommandRole {
    About,
    Quit,
}

fn default_true() -> bool {
    true
}

#[derive(Debug)]
pub struct ZintlWindowError {
    message: String,
}

impl ZintlWindowError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ZintlWindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ZintlWindowError {}

#[derive(Debug)]
pub struct ZintlAppError {
    message: String,
}

impl ZintlAppError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ZintlAppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for ZintlAppError {}

#[derive(Clone, Default)]
pub struct ZintlApi {
    pub app: Option<Arc<dyn ZintlAppApi>>,
    pub window: Option<Arc<dyn ZintlWindowApi>>,
}

#[derive(Clone, Default)]
struct ZintlApiState {
    app: Option<Arc<dyn ZintlAppApi>>,
    window: Option<Arc<dyn ZintlWindowApi>>,
}

#[op2]
async fn op_zintl_window_create(
    state: Rc<RefCell<OpState>>,
    #[serde] options: Option<ZintlWindowCreateOptions>,
) -> Result<ZintlWindowId, JsErrorBox> {
    window_host(&state)?
        .create_window(options.unwrap_or_default())
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
async fn op_zintl_window_set_bounds(
    state: Rc<RefCell<OpState>>,
    window_id: ZintlWindowId,
    #[serde] bounds: ZintlWindowBounds,
) -> Result<(), JsErrorBox> {
    window_host(&state)?
        .set_window_bounds(window_id, bounds)
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
async fn op_zintl_window_set_size(
    state: Rc<RefCell<OpState>>,
    window_id: ZintlWindowId,
    #[serde] size: ZintlWindowSize,
) -> Result<(), JsErrorBox> {
    window_host(&state)?
        .set_window_size(window_id, size)
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
async fn op_zintl_window_set_position(
    state: Rc<RefCell<OpState>>,
    window_id: ZintlWindowId,
    #[serde] position: ZintlWindowPosition,
) -> Result<(), JsErrorBox> {
    window_host(&state)?
        .set_window_position(window_id, position)
        .await
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

pub(super) fn app_host(state: &OpState) -> Result<Arc<dyn ZintlAppApi>, JsErrorBox> {
    let api = state.borrow::<ZintlApiState>();
    let Some(host) = api.app.as_ref() else {
        return Err(JsErrorBox::generic("app API is not registered"));
    };
    Ok(host.clone())
}

fn window_host(state: &Rc<RefCell<OpState>>) -> Result<Arc<dyn ZintlWindowApi>, JsErrorBox> {
    let state = state.borrow();
    let api = state.borrow::<ZintlApiState>();
    let Some(host) = api.window.as_ref() else {
        return Err(JsErrorBox::generic(
            "app.createWindow API is not registered",
        ));
    };
    Ok(host.clone())
}

deno_runtime::deno_core::extension!(
    zintl,
    ops = [
        op_zintl_window_create,
        op_zintl_window_set_bounds,
        op_zintl_window_set_size,
        op_zintl_window_set_position,
        app::op_zintl_app_next_event,
        app::op_zintl_app_set_commands,
    ],
    esm_entry_point = "ext:zintl/app.ts",
    esm = [
        "ext:zintl/app.ts" = "../../libs/app.ts",
        "ext:zintl/window.ts" = "../../libs/window.ts",
    ],
    options = {
        app: Option<Arc<dyn ZintlAppApi>>,
        window: Option<Arc<dyn ZintlWindowApi>>,
    },
    state = |state, options| {
        // Initialize a global app state
        state.put(ZintlApiState {
            app: options.app,
            window: options.window,
        });
    },
);

pub(crate) fn extension(api: ZintlApi) -> Extension {
    zintl::init(api.app, api.window)
}
