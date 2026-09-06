use crate::ffi::*;
use std::ffi::c_char;

#[macro_export]
macro_rules! msg_send {
    {$rec:expr, $sel:expr, ($($arg:ident : $typ:ty),*) => $ret:ty} => {
        // SAFETY: Each wrapper fixes the Objective-C ABI signature used by its caller.
        let function: unsafe extern "C" fn($crate::ffi::Id, $crate::ffi::Sel, $($typ),*) -> $ret =
            unsafe { transmute($crate::ffi::objc_msgSend as $crate::ffi::Imp) };
        unsafe { function($rec, $sel, $($arg),*) }
    };
}

#[macro_export]
macro_rules! sel {
    ($reg:ident) => {
        // SAFETY: TODO
        unsafe { $crate::ffi::sel_registerName(format!(b"{}\0" stringify!($reg))) }
    }
}

#[macro_export]
macro_rules! class {
    ($reg:ident) => {
        // SAFETY: TODO
        unsafe { $crate::ffi::objc_getClass(format!(b"{}\0" stringify!($reg))) }
    }
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
