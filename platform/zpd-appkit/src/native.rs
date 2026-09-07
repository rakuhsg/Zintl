#![allow(dead_code)]

use std::ffi::{c_int, c_long};

use zpd_objc::{Id, Strong, msg_send};

pub type CFloat = f64;
pub type Integer = c_long;
pub type Boolean = i8;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Point {
    pub x: CFloat,
    pub y: CFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Size {
    pub width: CFloat,
    pub height: CFloat,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub origin: Point,
    pub size: Size,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}
unsafe extern "C" {
    pub fn pthread_main_np() -> c_int;
}

pub fn nsstring(value: &str) -> Strong {
    // SAFETY: NSString copies the supplied UTF-8 bytes during initialization.
    unsafe {
        let allocated = msg_send!(zpd_objc::class!("NSString"), zpd_objc::sel!("alloc"), () => Id);
        Strong::from_retained(msg_send!(allocated, zpd_objc::sel!("initWithBytes:length:encoding:"), ((value.as_ptr()): *const u8, (value.len()): usize, (4): u64) => Id))
        .expect("NSString allocation failed")
    }
}

pub unsafe fn rust_string(value: Id) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: Both messages are valid for NSString.
    let length = unsafe {
        msg_send!(value, zpd_objc::sel!("lengthOfBytesUsingEncoding:"), ((4): u64) => usize)
    };
    let bytes = unsafe {
        msg_send!(value, zpd_objc::sel!("cStringUsingEncoding:"), ((4): u64) => *const u8)
    };
    if bytes.is_null() {
        return String::new();
    }
    // SAFETY: NSString exposes at least `length` UTF-8 bytes at this pointer.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(bytes, length) }).into_owned()
}

pub fn alloc_init(class: zpd_objc::Class) -> Strong {
    // SAFETY: The named Objective-C class implements alloc/init.
    unsafe {
        let value = msg_send!(class, zpd_objc::sel!("alloc"), () => Id);
        Strong::from_retained(msg_send!(value, zpd_objc::sel!("init"), () => Id))
            .expect("Objective-C allocation failed")
    }
}
