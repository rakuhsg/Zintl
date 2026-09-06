pub mod ffi;
mod obj;
mod runtime;

pub use ffi::{Class, Id, ObjcSuper, Sel};
pub use obj::*;
pub use runtime::*;

pub const NIL: Id = std::ptr::null_mut();
