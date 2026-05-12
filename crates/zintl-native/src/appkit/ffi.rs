use std::ffi::c_void;

#[cfg(feature = "wgpu")]
use crate::geometry::{PhysicalSize, Rect};

#[repr(C)]
pub struct AppCallback {
    pub on_init: unsafe extern "C" fn(*const c_void),
    pub perform: unsafe extern "C" fn(*const c_void),
    pub will_terminate: unsafe extern "C" fn(*const c_void),
}

#[repr(C)]
#[allow(dead_code)]
pub struct WindowCallback {
    pub on_appear: unsafe extern "C" fn(*const c_void),
    pub will_close: unsafe extern "C" fn(*const c_void),
}

unsafe extern "C" {
    pub fn zintlappkit_init(ud: *const c_void, cb: *const AppCallback);
    pub fn zintlappkit_schedule();
    pub fn zintlappkit_run();
    #[allow(dead_code)]
    pub fn zintlappkit_destroy();
    pub fn zintlappkit_create_window() -> *const c_void;
    pub fn zintlappkit_show_window(ptr: *const c_void);
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
}

#[cfg(feature = "wgpu")]
pub unsafe fn wgpu_surface_drawable_size(surface: *const c_void) -> PhysicalSize {
    let mut width = 0;
    let mut height = 0;
    // SAFETY: The caller guarantees `surface` is a valid AppKit surface pointer;
    // the out pointers are stack locals valid for this call.
    unsafe {
        zintlappkit_wgpu_surface_drawable_size(surface, &mut width, &mut height);
    }
    PhysicalSize { width, height }
}
