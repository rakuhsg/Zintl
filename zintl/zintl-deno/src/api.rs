use std::sync::Arc;

use deno_error::JsErrorBox;
use deno_runtime::deno_core;
use deno_runtime::deno_core::Extension;
use deno_runtime::deno_core::OpState;
use deno_runtime::deno_core::op2;

pub trait ZintlWindow: Send + Sync + 'static {
    fn create_window(&self) -> Result<(), ZintlWindowError>;
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

#[derive(Clone, Default)]
pub struct ZintlApi {
    pub window: Option<Arc<dyn ZintlWindow>>,
}

#[derive(Clone, Default)]
struct ZintlWindowApi {
    host: Option<Arc<dyn ZintlWindow>>,
}

#[op2(fast)]
fn op_zintl_window_create(state: &mut OpState) -> Result<(), JsErrorBox> {
    let api = state.borrow::<ZintlWindowApi>();
    let Some(host) = api.host.as_ref() else {
        return Err(JsErrorBox::generic("Zintl.window API is not registered"));
    };

    host.create_window()
        .map_err(|error| JsErrorBox::generic(error.to_string()))
}

deno_runtime::deno_core::extension!(
    zintl,
    ops = [op_zintl_window_create],
    esm_entry_point = "ext:zintl/window.ts",
    esm = ["ext:zintl/window.ts" = "../../api/window.ts"],
    options = {
        window: Option<Arc<dyn ZintlWindow>>,
    },
    state = |state, options| {
        state.put(ZintlWindowApi {
            host: options.window,
        });
    },
);

pub(crate) fn extension(api: ZintlApi) -> Extension {
    zintl::init(api.window)
}
