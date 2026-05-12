use std::ffi::c_void;

#[repr(C)]
pub struct AppCallback {
    pub on_init: unsafe extern "C" fn(*const c_void),
    pub perform: unsafe extern "C" fn(*const c_void),
    pub will_terminate: unsafe extern "C" fn(*const c_void),
}

#[repr(C)]
pub struct WindowCallback {
    pub on_appear: unsafe extern "C" fn(*const c_void),
    pub will_close: unsafe extern "C" fn(*const c_void),
}

unsafe extern "C" {
    pub fn zintlappkit_init(ud: *const c_void, cb: *const AppCallback);
    pub fn zintlappkit_schedule();
    pub fn zintlappkit_run();
    pub fn zintlappkit_destroy();
    pub fn zintlappkit_create_window() -> *const c_void;
    pub fn zintlappkit_show_window(ptr: *const c_void);
    pub fn zintlappkit_destroy_window(ptr: *const c_void);
}
