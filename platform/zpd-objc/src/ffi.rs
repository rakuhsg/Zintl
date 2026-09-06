use std::ffi::{c_char, c_long, c_void};

pub type Id = *mut c_void;
pub type Class = *mut c_void;
pub type Sel = *mut c_void;
pub type Imp = unsafe extern "C" fn();
pub type CFloat = f64;
pub type Integer = c_long;
pub type UInteger = u64;
pub type Boolean = i8;

#[repr(C)]
pub struct ObjcSuper {
    pub receiver: Id,
    pub superclass: Class,
}

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
    pub fn class_getInstanceMethod(cls: Class, name: Sel) -> *mut c_void;
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
    pub fn method_getImplementation(method: *mut c_void) -> Imp;
    pub fn object_getClass(object: Id) -> Class;
    pub fn objc_retain(object: Id) -> Id;
    pub fn objc_release(object: Id);
    pub fn objc_initWeak(location: *mut Id, object: Id) -> Id;
    pub fn objc_destroyWeak(location: *mut Id);
    pub fn objc_loadWeakRetained(location: *mut Id) -> Id;
    pub fn objc_msgSend();
    pub fn objc_msgSendSuper();
}
