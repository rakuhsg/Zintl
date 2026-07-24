use std::ffi::{c_char, c_void};

use crate::geometry::Rect;

#[repr(C)]
pub struct AppCallback {
    pub on_launch: unsafe extern "C" fn(*const c_void),
    pub perform: unsafe extern "C" fn(*const c_void),
    pub will_terminate: unsafe extern "C" fn(*const c_void),
}

#[repr(C)]
pub struct WindowCallback {
    pub did_create: unsafe extern "C" fn(*const c_void),
    pub will_close: unsafe extern "C" fn(*const c_void),
    pub did_close: unsafe extern "C" fn(*const c_void),
    pub did_click: unsafe extern "C" fn(*const c_void),
}

pub type CommandCallback =
    unsafe extern "C" fn(user_data: *const c_void, command_id: *const c_char);
pub type CommandRelease = unsafe extern "C" fn(user_data: *const c_void);

unsafe extern "C" {
    pub fn zintlappkit_init(ud: *const c_void, cb: *const AppCallback);
    pub fn zintlappkit_schedule();
    pub fn zintlappkit_run();
    pub fn zintlappkit_destroy();
    pub fn zintlappkit_create_window(
        user_data: *const c_void,
        callback: *const WindowCallback,
    ) -> *const c_void;
    pub fn zintlappkit_show_window(ptr: *const c_void);
    pub fn zintlappkit_window_set_bounds(ptr: *const c_void, bounds: Rect);
    pub fn zintlappkit_window_set_size(ptr: *const c_void, width: f64, height: f64);
    pub fn zintlappkit_window_set_position(ptr: *const c_void, x: f64, y: f64);
    pub fn zintlappkit_set_commands(
        commands_json: *const c_char,
        user_data: *const c_void,
        callback: CommandCallback,
        release: CommandRelease,
    );
    pub fn zintlappkit_destroy_window(ptr: *const c_void);
    #[cfg(feature = "wgpu")]
    pub fn zintlappkit_create_wgpu_surface(window: *const c_void, rect: Rect) -> *const c_void;
    #[cfg(feature = "wgpu")]
    pub fn zintlappkit_destroy_wgpu_surface(surface: *const c_void);
    #[cfg(feature = "wgpu")]
    pub fn zintlappkit_wgpu_surface_set_rect(surface: *const c_void, rect: Rect);
    #[cfg(feature = "wgpu")]
    pub fn zintlappkit_wgpu_surface_drawable_size(
        surface: *const c_void,
        out_width: *mut u32,
        out_height: *mut u32,
    );
    #[cfg(feature = "wgpu")]
    pub fn zintlappkit_wgpu_surface_metal_layer(surface: *const c_void) -> *mut c_void;

    pub fn pthread_main_np() -> std::ffi::c_int;
}
