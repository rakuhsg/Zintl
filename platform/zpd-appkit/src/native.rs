#![allow(dead_code)]

use std::ffi::{c_char, c_int, c_long, c_uint, c_void};
use std::mem::{transmute, transmute_copy};
use std::ptr::NonNull;

pub type Id = *mut c_void;
pub type Class = *mut c_void;
pub type Sel = *mut c_void;
pub type Imp = unsafe extern "C" fn();
pub type CFloat = f64;
pub type Integer = c_long;
pub type UInteger = u64;
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

pub const NIL: Id = std::ptr::null_mut();

#[link(name = "objc")]
unsafe extern "C" {
    pub fn objc_getClass(name: *const c_char) -> Class;
    pub fn objc_allocateClassPair(
        superclass: Class,
        name: *const c_char,
        extra_bytes: usize,
    ) -> Class;
    pub fn objc_registerClassPair(cls: Class);
    pub fn class_addMethod(cls: Class, name: Sel, imp: Imp, types: *const c_char) -> bool;
    pub fn class_addIvar(
        cls: Class,
        name: *const c_char,
        size: usize,
        alignment: u8,
        types: *const c_char,
    ) -> bool;
    pub fn sel_registerName(name: *const c_char) -> Sel;
    pub fn object_getIvar(object: Id, ivar: *mut c_void) -> Id;
    pub fn object_setIvar(object: Id, ivar: *mut c_void, value: Id);
    pub fn class_getInstanceVariable(cls: Class, name: *const c_char) -> *mut c_void;
    pub fn ivar_getOffset(ivar: *mut c_void) -> isize;
    pub fn object_getClass(object: Id) -> Class;
    pub fn objc_retain(object: Id) -> Id;
    pub fn objc_release(object: Id);
    pub fn objc_initWeak(location: *mut Id, object: Id) -> Id;
    pub fn objc_destroyWeak(location: *mut Id);
    pub fn objc_loadWeakRetained(location: *mut Id) -> Id;
    fn objc_msgSend();
    fn objc_msgSendSuper();
}

pub unsafe fn get_pointer_ivar<T>(object: Id, name: *const c_char) -> *mut T {
    // SAFETY: The named ivar was registered as pointer-sized storage on this class.
    let ivar = unsafe { class_getInstanceVariable(object_getClass(object), name) };
    let offset = unsafe { ivar_getOffset(ivar) };
    unsafe { *object.cast::<u8>().offset(offset).cast::<*mut T>() }
}

pub unsafe fn set_pointer_ivar<T>(object: Id, name: *const c_char, value: *mut T) {
    // SAFETY: The named ivar was registered as pointer-sized storage on this class.
    let ivar = unsafe { class_getInstanceVariable(object_getClass(object), name) };
    let offset = unsafe { ivar_getOffset(ivar) };
    unsafe { *object.cast::<u8>().offset(offset).cast::<*mut T>() = value };
}

#[repr(C)]
pub struct Super {
    pub receiver: Id,
    pub superclass: Class,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}
#[link(name = "Foundation", kind = "framework")]
unsafe extern "C" {}
#[link(name = "QuartzCore", kind = "framework")]
unsafe extern "C" {}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    pub fn CFRunLoopGetCurrent() -> *mut c_void;
    pub fn CFRunLoopGetMain() -> *mut c_void;
    pub fn CFRunLoopWakeUp(run_loop: *mut c_void);
    pub fn CFRunLoopStop(run_loop: *mut c_void);
    pub fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    pub fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    pub fn CFRunLoopSourceCreate(
        allocator: *const c_void,
        order: Integer,
        context: *mut CFRunLoopSourceContext,
    ) -> *mut c_void;
    pub fn CFRunLoopSourceSignal(source: *mut c_void);
    pub fn CFRunLoopSourceInvalidate(source: *mut c_void);
    pub fn CFRelease(value: *const c_void);
    pub static kCFRunLoopCommonModes: *const c_void;
}

unsafe extern "C" {
    pub fn pthread_main_np() -> c_int;
}

#[repr(C)]
pub struct CFRunLoopSourceContext {
    pub version: Integer,
    pub info: *mut c_void,
    pub retain: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub release: Option<unsafe extern "C" fn(*const c_void)>,
    pub copy_description: Option<unsafe extern "C" fn(*const c_void) -> *const c_void>,
    pub equal: Option<unsafe extern "C" fn(*const c_void, *const c_void) -> Boolean>,
    pub hash: Option<unsafe extern "C" fn(*const c_void) -> usize>,
    pub schedule: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    pub cancel: Option<unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void)>,
    pub perform: Option<unsafe extern "C" fn(*mut c_void)>,
}

impl Default for CFRunLoopSourceContext {
    fn default() -> Self {
        // SAFETY: A zeroed Core Foundation source context is its documented default.
        unsafe { std::mem::zeroed() }
    }
}

#[inline]
pub fn class(name: &'static [u8]) -> Class {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: Callers provide a static NUL-terminated class name.
    unsafe { objc_getClass(name.as_ptr().cast()) }
}

#[inline]
pub fn sel(name: &'static [u8]) -> Sel {
    debug_assert_eq!(name.last(), Some(&0));
    // SAFETY: Callers provide a static NUL-terminated selector name.
    unsafe { sel_registerName(name.as_ptr().cast()) }
}

macro_rules! send {
    ($name:ident, $ret:ty, ($($arg:ident : $ty:ty),*)) => {
        #[inline]
        pub unsafe fn $name(receiver: Id, selector: Sel, $($arg: $ty),*) -> $ret {
            // SAFETY: Each wrapper fixes the Objective-C ABI signature used by its caller.
            let function: unsafe extern "C" fn(Id, Sel, $($ty),*) -> $ret =
                unsafe { transmute(objc_msgSend as Imp) };
            unsafe { function(receiver, selector, $($arg),*) }
        }
    };
}

send!(send_id, Id, ());
send!(send_id_id, Id, (a: Id));
send!(send_id_rect, Id, (a: Rect));
send!(send_id_rect_u64_u64_bool, Id, (a: Rect, b: UInteger, c: UInteger, d: bool));
send!(send_id_str, Id, (bytes: *const u8, len: usize, encoding: UInteger));
send!(send_void, (), ());
send!(send_void_id, (), (a: Id));
send!(send_void_bool, (), (a: bool));
send!(send_void_i64, (), (a: Integer));
send!(send_void_u64, (), (a: UInteger));
send!(send_void_f64, (), (a: f64));
send!(send_void_f32, (), (a: f32));
send!(send_void_rect, (), (a: Rect));
send!(send_void_rect_bool, (), (a: Rect, b: bool));
send!(send_void_size, (), (a: Size));
send!(send_void_point, (), (a: Point));
send!(send_bool, bool, ());
send!(send_bool_id, bool, (a: Id));
send!(send_u64, UInteger, ());
send!(send_f64, f64, ());
send!(send_rect, Rect, ());
send!(send_size, Size, ());
send!(send_id_i64, Id, (a: Integer));
send!(send_id_u64, Id, (a: UInteger));
send!(send_void_id_id, (), (a: Id, b: Id));
send!(send_void_id_i64, (), (a: Id, b: Integer));
send!(send_void_id_bool, (), (a: Id, b: bool));
send!(send_id_id_id, Id, (a: Id, b: Id));
send!(send_id_id_id_id, Id, (a: Id, b: Id, c: Id));
#[allow(clippy::too_many_arguments)]
pub unsafe fn send_constraint(
    receiver: Id,
    selector: Sel,
    first: Id,
    first_attribute: Integer,
    relation: Integer,
    second: Id,
    second_attribute: Integer,
    multiplier: f64,
    constant: f64,
) -> Id {
    // SAFETY: This signature matches NSLayoutConstraint's factory method on arm64.
    let function: unsafe extern "C" fn(Id, Sel, Id, Integer, Integer, Id, Integer, f64, f64) -> Id =
        unsafe { transmute(objc_msgSend as Imp) };
    unsafe {
        function(
            receiver,
            selector,
            first,
            first_attribute,
            relation,
            second,
            second_attribute,
            multiplier,
            constant,
        )
    }
}

pub unsafe fn send_application_event(receiver: Id, selector: Sel) -> Id {
    // SAFETY: This is NSEvent's application-defined event factory signature on arm64.
    let function: unsafe extern "C" fn(
        Id,
        Sel,
        UInteger,
        Point,
        UInteger,
        f64,
        Integer,
        Id,
        i16,
        Integer,
        Integer,
    ) -> Id = unsafe { transmute(objc_msgSend as Imp) };
    unsafe {
        function(
            receiver,
            selector,
            15,
            Point::default(),
            0,
            0.0,
            0,
            NIL,
            0,
            0,
            0,
        )
    }
}

pub unsafe fn send_super_void(receiver: Id, superclass: Class, selector: Sel) {
    let mut value = Super {
        receiver,
        superclass,
    };
    // SAFETY: The cast matches an Objective-C super send returning void.
    let function: unsafe extern "C" fn(*mut Super, Sel) =
        unsafe { transmute(objc_msgSendSuper as Imp) };
    unsafe { function(&mut value, selector) };
}

pub struct Strong(NonNull<c_void>);

impl Strong {
    pub unsafe fn from_retained(raw: Id) -> Option<Self> {
        NonNull::new(raw).map(Self)
    }

    pub unsafe fn retain(raw: Id) -> Option<Self> {
        if raw.is_null() {
            return None;
        }
        // SAFETY: The caller supplied a live Objective-C object.
        let raw = unsafe { objc_retain(raw) };
        unsafe { Self::from_retained(raw) }
    }

    pub fn as_ptr(&self) -> Id {
        self.0.as_ptr()
    }
}

impl Clone for Strong {
    fn clone(&self) -> Self {
        // SAFETY: self owns a live +1 reference.
        unsafe { Self::retain(self.as_ptr()).expect("retaining a live object cannot return nil") }
    }
}

impl Drop for Strong {
    fn drop(&mut self) {
        // SAFETY: This consumes exactly one owned Objective-C reference.
        unsafe { objc_release(self.as_ptr()) };
    }
}

pub fn nsstring(value: &str) -> Strong {
    // SAFETY: NSString copies the supplied UTF-8 bytes during initialization.
    unsafe {
        let allocated = send_id(class(b"NSString\0"), sel(b"alloc\0"));
        Strong::from_retained(send_id_str(
            allocated,
            sel(b"initWithBytes:length:encoding:\0"),
            value.as_ptr(),
            value.len(),
            4,
        ))
        .expect("NSString allocation failed")
    }
}

pub unsafe fn rust_string(value: Id) -> String {
    if value.is_null() {
        return String::new();
    }
    // SAFETY: Both messages are valid for NSString.
    let length_fn: unsafe extern "C" fn(Id, Sel, UInteger) -> usize =
        unsafe { transmute(objc_msgSend as Imp) };
    let length = unsafe { length_fn(value, sel(b"lengthOfBytesUsingEncoding:\0"), 4) };
    let bytes_fn: unsafe extern "C" fn(Id, Sel, UInteger) -> *const u8 =
        unsafe { transmute(objc_msgSend as Imp) };
    let bytes = unsafe { bytes_fn(value, sel(b"cStringUsingEncoding:\0"), 4) };
    if bytes.is_null() {
        return String::new();
    }
    // SAFETY: NSString exposes at least `length` UTF-8 bytes at this pointer.
    String::from_utf8_lossy(unsafe { std::slice::from_raw_parts(bytes, length) }).into_owned()
}

pub fn alloc_init(class_name: &'static [u8]) -> Strong {
    // SAFETY: The named Objective-C class implements alloc/init.
    unsafe {
        let value = send_id(class(class_name), sel(b"alloc\0"));
        Strong::from_retained(send_id(value, sel(b"init\0")))
            .expect("Objective-C allocation failed")
    }
}

pub fn imp<T: Copy>(value: T) -> Imp {
    assert_eq!(std::mem::size_of::<T>(), std::mem::size_of::<Imp>());
    // SAFETY: Objective-C IMP and the supplied extern function pointer have pointer size.
    unsafe { transmute_copy(&value) }
}

pub unsafe fn add_method<T: Copy>(
    class: Class,
    name: &'static [u8],
    method: T,
    types: &'static [u8],
) {
    // SAFETY: The class is unregistered and names/encoding are NUL-terminated.
    assert!(unsafe { class_addMethod(class, sel(name), imp(method), types.as_ptr().cast()) });
}

pub fn bool_value(value: bool) -> Boolean {
    i8::from(value)
}

pub fn uint(value: c_uint) -> UInteger {
    u64::from(value)
}
