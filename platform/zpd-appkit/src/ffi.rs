use std::ffi::c_void;

use crate::geometry::Rect;
use crate::string::{NativeOptionalString, NativeString, NativeStringCallback};

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
    pub release: WindowRelease,
}

pub type WindowRelease = unsafe extern "C" fn(user_data: *const c_void);
pub type ControlAction = unsafe extern "C" fn(user_data: *const c_void);
pub type ControlRelease = unsafe extern "C" fn(user_data: *const c_void);
pub type TextFieldChangeCallback =
    unsafe extern "C" fn(user_data: *const c_void, value: NativeString);
pub type CommandCallback = unsafe extern "C" fn(user_data: *const c_void, command_id: NativeString);
pub type CommandRelease = unsafe extern "C" fn(user_data: *const c_void);
pub type SidebarSelectionCallback =
    unsafe extern "C" fn(user_data: *const c_void, item_id: NativeString);
pub type SidebarRelease = unsafe extern "C" fn(user_data: *const c_void);
pub type RunLoopSourcePerform = unsafe extern "C" fn(user_data: *const c_void);

unsafe extern "C" {
    pub fn zintlappkit_init(ud: *const c_void, cb: *const AppCallback);
    pub fn zintlappkit_schedule();
    pub fn zintlappkit_run();
    pub fn zintlappkit_stop();
    pub fn zintlappkit_destroy();
    pub fn zintlappkit_application_run_loop() -> *const c_void;
    pub fn zintlappkit_run_loop_is_current(run_loop: *const c_void) -> bool;
    pub fn zintlappkit_run_loop_stop(run_loop: *const c_void);
    pub fn zintlappkit_run_loop_source_create(
        run_loop: *const c_void,
        user_data: *const c_void,
        perform: RunLoopSourcePerform,
    ) -> *const c_void;
    pub fn zintlappkit_run_loop_source_signal(source: *const c_void);
    pub fn zintlappkit_run_loop_source_destroy(source: *const c_void);
    pub fn zintlappkit_create_window(
        user_data: *const c_void,
        callback: *const WindowCallback,
    ) -> *const c_void;
    pub fn zintlappkit_show_window(ptr: *const c_void);
    pub fn zintlappkit_window_set_title(ptr: *const c_void, title: NativeString);
    pub fn zintlappkit_window_set_bounds(ptr: *const c_void, bounds: Rect);
    pub fn zintlappkit_window_set_size(ptr: *const c_void, width: f64, height: f64);
    pub fn zintlappkit_window_set_position(ptr: *const c_void, x: f64, y: f64);
    pub fn zintlappkit_window_content_view(ptr: *const c_void) -> *mut c_void;
    pub fn zintlappkit_window_set_sidebar(
        window: *const c_void,
        sidebar_json: NativeString,
        user_data: *const c_void,
        callback: SidebarSelectionCallback,
        release: SidebarRelease,
    ) -> bool;
    pub fn zintlappkit_window_clear_sidebar(window: *const c_void);
    pub fn zintlappkit_create_view(frame: Rect) -> *mut c_void;
    pub fn zintlappkit_release_view(view: *const c_void);
    pub fn zintlappkit_view_add_subview(parent: *const c_void, child: *const c_void);
    pub fn zintlappkit_view_remove_from_superview(view: *const c_void);
    pub fn zintlappkit_view_set_frame(view: *const c_void, frame: Rect);
    pub fn zintlappkit_view_set_translates_autoresizing_mask_into_constraints(
        view: *const c_void,
        enabled: bool,
    );
    pub fn zintlappkit_create_button(title: NativeString) -> *mut c_void;
    pub fn zintlappkit_button_set_title(button: *const c_void, title: NativeString);
    pub fn zintlappkit_button_set_action(
        button: *const c_void,
        user_data: *const c_void,
        action: ControlAction,
        release: ControlRelease,
    );
    pub fn zintlappkit_button_clear_action(button: *const c_void);
    pub fn zintlappkit_create_text_field(value: NativeString, label: bool) -> *mut c_void;
    pub fn zintlappkit_text_field_set_string_value(text_field: *const c_void, value: NativeString);
    pub fn zintlappkit_text_field_get_string_value(
        text_field: *const c_void,
        user_data: *mut c_void,
        callback: NativeStringCallback,
    );
    pub fn zintlappkit_text_field_set_placeholder_string(
        text_field: *const c_void,
        value: NativeOptionalString,
    );
    pub fn zintlappkit_text_field_set_editable(text_field: *const c_void, editable: bool);
    pub fn zintlappkit_text_field_set_selectable(text_field: *const c_void, selectable: bool);
    pub fn zintlappkit_text_field_set_change_handler(
        text_field: *const c_void,
        user_data: *const c_void,
        callback: TextFieldChangeCallback,
        release: ControlRelease,
    );
    pub fn zintlappkit_text_field_clear_change_handler(text_field: *const c_void);
    pub fn zintlappkit_layout_constraint_create(
        first_view: *const c_void,
        first_attribute: i32,
        relation: i32,
        second_view: *const c_void,
        second_attribute: i32,
        multiplier: f64,
        constant: f64,
    ) -> *mut c_void;
    pub fn zintlappkit_layout_constraint_set_active(constraint: *const c_void, active: bool);
    pub fn zintlappkit_layout_constraint_set_priority(constraint: *const c_void, priority: f32);
    pub fn zintlappkit_release_layout_constraint(constraint: *const c_void);
    pub fn zintlappkit_set_commands(
        commands_json: NativeString,
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
    #[cfg(feature = "wgpu")]
    pub fn zintlappkit_wgpu_surface_view(surface: *const c_void) -> *mut c_void;

    pub fn pthread_main_np() -> std::ffi::c_int;
}
