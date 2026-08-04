pub mod api;
pub mod module;
pub mod runtime;
mod sys;

pub use module::MainModule;
pub use runtime::{DenoRuntime, DenoRuntimeError, DenoRuntimeOptions};

pub(crate) const WEBGPU_FEATURE_NAME: &str = deno_runtime::deno_webgpu::UNSTABLE_FEATURE_NAME;
pub(crate) const ZINTL_DENO_SNAPSHOT: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/ZINTL_DENO_SNAPSHOT.bin"));
