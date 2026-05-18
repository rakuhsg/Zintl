use std::sync::Arc;

use deno_error::JsErrorBox;
use deno_runtime::deno_core;
use deno_runtime::deno_core::Extension;
use deno_runtime::deno_core::OpState;
use deno_runtime::deno_core::op2;
use serde::Deserialize;

mod app;

pub use app::ZintlAppEvent;

pub trait ZintlWindowApi: Send + Sync + 'static {
    fn create_window(
        &self,
        options: ZintlWindowCreateOptions,
    ) -> Result<ZintlWindowId, ZintlWindowError>;
    fn set_window_bounds(
        &self,
        window_id: ZintlWindowId,
        bounds: ZintlWindowBounds,
    ) -> Result<(), ZintlWindowError>;
    fn set_window_size(
        &self,
        window_id: ZintlWindowId,
        size: ZintlWindowSize,
    ) -> Result<(), ZintlWindowError>;
    fn set_window_position(
        &self,
        window_id: ZintlWindowId,
        position: ZintlWindowPosition,
    ) -> Result<(), ZintlWindowError>;
    fn set_window_commands(
        &self,
        window_id: ZintlWindowId,
        commands: ZintlWindowCommandSet,
    ) -> Result<(), ZintlWindowError>;
}

pub trait ZintlAppApi: Send + Sync + 'static {
    fn take_event(&self) -> Result<Option<ZintlAppEvent>, ZintlAppError>;
}

pub type ZintlWindowId = u32;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ZintlWindowCreateOptions {
    pub bounds: Option<ZintlWindowBounds>,
    pub size: Option<ZintlWindowSize>,
    pub position: Option<ZintlWindowPosition>,
    pub commands: Option<ZintlWindowCommandSet>,
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
pub struct ZintlWindowCommandSet {
    #[serde(default, rename = "appMenu")]
    pub app_menu: Option<ZintlWindowAppMenu>,
    #[serde(default)]
    pub menus: Vec<ZintlWindowCommandMenu>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ZintlWindowAppMenu {
    pub items: Vec<ZintlWindowCommandItem>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ZintlWindowCommandMenu {
    pub title: String,
    pub items: Vec<ZintlWindowCommandItem>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ZintlWindowCommandItem {
    pub id: Option<String>,
    pub title: String,
    pub role: Option<ZintlWindowCommandRole>,
    pub key: Option<String>,
    #[serde(default)]
    pub modifiers: Vec<ZintlWindowCommandModifier>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZintlWindowCommandModifier {
    Cmd,
    Ctrl,
    Alt,
    Shift,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ZintlWindowCommandRole {
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
fn op_zintl_window_create(
    state: &mut OpState,
    #[serde] options: Option<ZintlWindowCreateOptions>,
) -> Result<ZintlWindowId, JsErrorBox> {
    window_host(state)?
        .create_window(options.unwrap_or_default())
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
fn op_zintl_window_set_bounds(
    state: &mut OpState,
    window_id: ZintlWindowId,
    #[serde] bounds: ZintlWindowBounds,
) -> Result<(), JsErrorBox> {
    window_host(state)?
        .set_window_bounds(window_id, bounds)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
fn op_zintl_window_set_size(
    state: &mut OpState,
    window_id: ZintlWindowId,
    #[serde] size: ZintlWindowSize,
) -> Result<(), JsErrorBox> {
    window_host(state)?
        .set_window_size(window_id, size)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
fn op_zintl_window_set_position(
    state: &mut OpState,
    window_id: ZintlWindowId,
    #[serde] position: ZintlWindowPosition,
) -> Result<(), JsErrorBox> {
    window_host(state)?
        .set_window_position(window_id, position)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

#[op2]
fn op_zintl_window_set_commands(
    state: &mut OpState,
    window_id: ZintlWindowId,
    #[serde] commands: ZintlWindowCommandSet,
) -> Result<(), JsErrorBox> {
    window_host(state)?
        .set_window_commands(window_id, commands)
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

pub(super) fn app_host(state: &mut OpState) -> Result<Arc<dyn ZintlAppApi>, JsErrorBox> {
    let api = state.borrow::<ZintlApiState>();
    let Some(host) = api.app.as_ref() else {
        return Err(JsErrorBox::generic("app API is not registered"));
    };
    Ok(host.clone())
}

fn window_host(state: &mut OpState) -> Result<Arc<dyn ZintlWindowApi>, JsErrorBox> {
    let api = state.borrow::<ZintlApiState>();
    let Some(host) = api.window.as_ref() else {
        return Err(JsErrorBox::generic("Zintl.window API is not registered"));
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
        op_zintl_window_set_commands,
        app::op_zintl_app_event_bus_poll,
    ],
    esm_entry_point = "ext:zintl/window.ts",
    esm = [
        "ext:zintl/app.ts" = "../../libs/app.ts",
        "ext:zintl/window.ts" = "../../libs/window.ts",
    ],
    options = {
        app: Option<Arc<dyn ZintlAppApi>>,
        window: Option<Arc<dyn ZintlWindowApi>>,
    },
    state = |state, options| {
        state.put(ZintlApiState {
            app: options.app,
            window: options.window,
        });
    },
);

pub(crate) fn extension(api: ZintlApi) -> Extension {
    zintl::init(api.app, api.window)
}
