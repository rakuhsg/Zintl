//! Safe, main-thread-aware Rust ownership wrappers for AppKit.

#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("zpd-appkit supports Apple Silicon macOS only");

mod actor;
pub mod geometry;
mod native;
pub mod runloop;
pub mod ui;
