//! Safe, main-thread-aware Rust ownership wrappers for Zintl's AppKit FFI.
//!
//! The final application must link `ZintlAppkitSupport`. Raw native handles
//! and callback pointers remain private to this crate.

mod ffi;
pub mod geometry;
pub mod runloop;
pub mod ui;
