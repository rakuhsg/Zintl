use crate::ffi::*;
use std::ffi::c_char;

#[macro_export]
macro_rules! msg_send {
    {$rec:expr, $sel:expr, ($(($arg:expr): $typ:ty),* $(,)?) => $ret:ty} => {{
        // SAFETY: The caller is responsible for matching the Objective-C method ABI.
        let function: unsafe extern "C" fn($crate::ffi::Id, $crate::ffi::Sel, $($typ),*) -> $ret =
            ::std::mem::transmute($crate::ffi::objc_msgSend as $crate::ffi::Imp);
        function($rec, $sel, $($arg),*)
    }};
    {$rec:expr, $sel:expr, ($($arg:ident : $typ:ty),*) => $ret:ty} => {{
        // SAFETY: Each wrapper fixes the Objective-C ABI signature used by its caller.
        let function: unsafe extern "C" fn($crate::ffi::Id, $crate::ffi::Sel, $($typ),*) -> $ret =
            unsafe { ::std::mem::transmute($crate::ffi::objc_msgSend as $crate::ffi::Imp) };
        unsafe { function($rec, $sel, $($arg),*) }
    }};
}

#[macro_export]
macro_rules! sel {
    ($name:literal) => {{
        // SAFETY: concat! produces a static NUL-terminated selector name.
        unsafe { $crate::ffi::sel_registerName(concat!($name, "\0").as_ptr().cast()) }
    }};
}

#[macro_export]
macro_rules! class {
    ($name:literal) => {{
        // SAFETY: concat! produces a static NUL-terminated class name.
        unsafe { $crate::ffi::objc_getClass(concat!($name, "\0").as_ptr().cast()) }
    }};
}

#[macro_export]
macro_rules! msg_send_super {
    {$receiver:expr, $superclass:expr, $selector:expr, ($(($arg:expr): $typ:ty),* $(,)?) => $ret:ty} => {{
        let mut value = $crate::ObjcSuper {
            receiver: $receiver,
            superclass: $superclass,
        };
        // SAFETY: The caller is responsible for matching the Objective-C method ABI.
        let function: unsafe extern "C" fn(
            *mut $crate::ObjcSuper,
            $crate::ffi::Sel,
            $($typ),*
        ) -> $ret = ::std::mem::transmute(
            $crate::ffi::objc_msgSendSuper as $crate::ffi::Imp
        );
        function(&mut value, $selector, $($arg),*)
    }};
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
