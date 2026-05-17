use deno_runtime::deno_core::error::CoreError;
use deno_runtime::deno_core::error::JsError;

pub mod api;
pub mod module;
pub mod runtime;
mod sys;

pub use module::MainModule;
pub use runtime::DenoRuntime;

pub(crate) const WEBGPU_FEATURE_NAME: &str = deno_runtime::deno_webgpu::UNSTABLE_FEATURE_NAME;
pub(crate) const ZINTL_DENO_SNAPSHOT: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/ZINTL_DENO_SNAPSHOT.bin"));

#[derive(Clone, Default)]
pub struct DenoRuntimeOptions {
    pub api: api::ZintlApi,
}

#[derive(Debug)]
pub enum DenoRuntimeError {
    Core(CoreError),
    LoadEvent(Box<JsError>),
}

impl std::fmt::Display for DenoRuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DenoRuntimeError::Core(error) => write!(f, "{error}"),
            DenoRuntimeError::LoadEvent(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for DenoRuntimeError {}

impl From<CoreError> for DenoRuntimeError {
    fn from(error: CoreError) -> Self {
        DenoRuntimeError::Core(error)
    }
}

impl From<Box<JsError>> for DenoRuntimeError {
    fn from(error: Box<JsError>) -> Self {
        DenoRuntimeError::LoadEvent(error)
    }
}
