use std::ffi::c_void;

#[repr(C)]
pub struct AppContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct DispatcherQueue {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Window {
    _private: [u8; 0],
}

#[repr(C)]
pub struct Element {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct StringRef {
    pub data: *const u8,
    pub length: usize,
}

impl StringRef {
    pub fn new(value: &str) -> Self {
        Self {
            data: value.as_ptr(),
            length: value.len(),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Thickness {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct GridLength {
    pub kind: i32,
    pub value: f64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MenuFlyoutItem {
    pub kind: i32,
    pub id: StringRef,
    pub title: StringRef,
    pub key: StringRef,
    pub modifiers: u32,
    pub enabled: bool,
    pub children: *const MenuFlyoutItem,
    pub children_length: usize,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MenuBarItem {
    pub title: StringRef,
    pub items: *const MenuFlyoutItem,
    pub items_length: usize,
}

pub type LaunchFn = unsafe extern "C" fn(*const AppContext, *const c_void);
pub type InvokeFn = unsafe extern "C" fn(*const c_void);
pub type StringFn = unsafe extern "C" fn(*const c_void, StringRef);
pub type ReleaseFn = unsafe extern "C" fn(*const c_void);

unsafe extern "C" {
    pub fn zpd_winui3_application_run(
        data: *const c_void,
        launch: LaunchFn,
        release: ReleaseFn,
    ) -> i32;
    pub fn zpd_winui3_app_dispatcher(context: *const AppContext) -> *mut DispatcherQueue;
    pub fn zpd_winui3_window_create(context: *const AppContext) -> *mut Window;
    pub fn zpd_winui3_dispatcher_clone(dispatcher: *const DispatcherQueue) -> *mut DispatcherQueue;
    pub fn zpd_winui3_dispatcher_release(dispatcher: *mut DispatcherQueue);
    pub fn zpd_winui3_dispatcher_try_enqueue(
        dispatcher: *const DispatcherQueue,
        priority: i32,
        data: *const c_void,
        invoke: InvokeFn,
        release: ReleaseFn,
    ) -> bool;
    pub fn zpd_winui3_window_release(window: *mut Window);
    pub fn zpd_winui3_window_set_title(window: *mut Window, title: StringRef) -> i32;
    pub fn zpd_winui3_window_resize(window: *mut Window, width: i32, height: i32) -> i32;
    pub fn zpd_winui3_window_activate(window: *mut Window) -> i32;
    pub fn zpd_winui3_window_close(window: *mut Window) -> i32;
    pub fn zpd_winui3_window_set_content(window: *mut Window, element: *const Element) -> i32;
    pub fn zpd_winui3_window_extend_content_into_title_bar(
        window: *mut Window,
        enabled: bool,
    ) -> i32;
    pub fn zpd_winui3_window_set_title_bar(window: *mut Window, element: *const Element) -> i32;
    pub fn zpd_winui3_window_set_backdrop(window: *mut Window, backdrop: i32) -> i32;
    pub fn zpd_winui3_window_set_menu_bar(
        window: *mut Window,
        menus: *const MenuBarItem,
        length: usize,
        data: *const c_void,
        invoke: StringFn,
        release: ReleaseFn,
    ) -> i32;
    pub fn zpd_winui3_window_clear_menu_bar(window: *mut Window) -> i32;
    pub fn zpd_winui3_button_create(title: StringRef) -> *mut Element;
    pub fn zpd_winui3_button_set_title(button: *mut Element, title: StringRef) -> i32;
    pub fn zpd_winui3_button_set_enabled(button: *mut Element, enabled: bool) -> i32;
    pub fn zpd_winui3_button_set_click_handler(
        button: *mut Element,
        data: *const c_void,
        invoke: InvokeFn,
        release: ReleaseFn,
    ) -> i32;
    pub fn zpd_winui3_button_clear_click_handler(button: *mut Element) -> i32;
    pub fn zpd_winui3_text_create(text: StringRef) -> *mut Element;
    pub fn zpd_winui3_text_set_text(text: *mut Element, value: StringRef) -> i32;
    pub fn zpd_winui3_text_set_wrapping(text: *mut Element, enabled: bool) -> i32;
    pub fn zpd_winui3_text_set_alignment(text: *mut Element, alignment: i32) -> i32;
    pub fn zpd_winui3_text_field_create(value: StringRef) -> *mut Element;
    pub fn zpd_winui3_text_field_set_value(field: *mut Element, value: StringRef) -> i32;
    pub fn zpd_winui3_text_field_get_value(
        field: *const Element,
        data: *const c_void,
        receive: StringFn,
    ) -> i32;
    pub fn zpd_winui3_text_field_set_placeholder(field: *mut Element, value: StringRef) -> i32;
    pub fn zpd_winui3_text_field_set_read_only(field: *mut Element, read_only: bool) -> i32;
    pub fn zpd_winui3_text_field_set_change_handler(
        field: *mut Element,
        data: *const c_void,
        invoke: StringFn,
        release: ReleaseFn,
    ) -> i32;
    pub fn zpd_winui3_text_field_clear_change_handler(field: *mut Element) -> i32;
    pub fn zpd_winui3_stack_panel_create(orientation: i32, spacing: f64) -> *mut Element;
    pub fn zpd_winui3_panel_append(panel: *mut Element, child: *const Element) -> i32;
    pub fn zpd_winui3_panel_clear(panel: *mut Element) -> i32;
    pub fn zpd_winui3_grid_create() -> *mut Element;
    pub fn zpd_winui3_grid_set_rows(
        grid: *mut Element,
        rows: *const GridLength,
        length: usize,
    ) -> i32;
    pub fn zpd_winui3_grid_set_columns(
        grid: *mut Element,
        columns: *const GridLength,
        length: usize,
    ) -> i32;
    pub fn zpd_winui3_grid_add(
        grid: *mut Element,
        child: *const Element,
        row: i32,
        column: i32,
        row_span: i32,
        column_span: i32,
    ) -> i32;
    pub fn zpd_winui3_element_release(element: *mut Element);
    pub fn zpd_winui3_element_set_margin(element: *mut Element, margin: Thickness) -> i32;
    pub fn zpd_winui3_element_set_width(element: *mut Element, width: f64) -> i32;
    pub fn zpd_winui3_element_set_height(element: *mut Element, height: f64) -> i32;
    pub fn zpd_winui3_element_set_horizontal_alignment(
        element: *mut Element,
        alignment: i32,
    ) -> i32;
    pub fn zpd_winui3_element_set_vertical_alignment(element: *mut Element, alignment: i32) -> i32;
}
