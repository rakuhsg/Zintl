//! Main-thread-safe Rust bindings for WinUI 3.

#![cfg(target_os = "windows")]

mod ffi;

use std::cell::RefCell;
use std::ffi::c_void;
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::rc::Rc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub i32);

impl std::fmt::Display for Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "WinUI 3 operation failed with HRESULT 0x{:08X}",
            self.0
        )
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

fn check(status: i32) -> Result<()> {
    if status >= 0 {
        Ok(())
    } else {
        Err(Error(status))
    }
}

fn abort_on_panic(body: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(body)).is_err() {
        std::process::abort();
    }
}

pub struct Application;

impl Application {
    /// Starts WinUI and invokes `launched` on its UI thread.
    pub fn run<F, S>(launched: F) -> Result<()>
    where
        F: FnOnce(&AppContext) -> S + 'static,
        S: 'static,
    {
        struct State<F, S> {
            callback: Option<F>,
            application_state: Option<S>,
        }

        unsafe extern "C" fn launch<F, S>(context: *const ffi::AppContext, data: *const c_void)
        where
            F: FnOnce(&AppContext) -> S,
        {
            abort_on_panic(|| {
                // SAFETY: Native code invokes this with the allocation transferred below.
                let state = unsafe { &mut *data.cast_mut().cast::<State<F, S>>() };
                let callback = state.callback.take().expect("launch callback ran twice");
                state.application_state = Some(callback(&AppContext {
                    raw: context,
                    _main_thread: PhantomData,
                }));
            });
        }

        unsafe extern "C" fn release<F, S>(data: *const c_void) {
            if !data.is_null() {
                // SAFETY: Native code releases the one Box transferred to it exactly once.
                unsafe { drop(Box::from_raw(data.cast_mut().cast::<State<F, S>>())) };
            }
        }

        let state: *mut State<F, S> = Box::into_raw(Box::new(State {
            callback: Some(launched),
            application_state: None,
        }));
        // SAFETY: `state` remains owned by native code until `release` is called.
        check(unsafe {
            ffi::zpd_winui3_application_run(state.cast(), launch::<F, S>, release::<F, S>)
        })
    }
}

pub struct AppContext<'application> {
    raw: *const ffi::AppContext,
    _main_thread: PhantomData<&'application Rc<()>>,
}

impl AppContext<'_> {
    pub fn dispatcher_queue(&self) -> Result<DispatcherQueue> {
        // SAFETY: The context is valid for the synchronous launch callback.
        let raw = unsafe { ffi::zpd_winui3_app_dispatcher(self.raw) };
        Ok(DispatcherQueue {
            raw: NonNull::new(raw).ok_or(Error(-1))?,
        })
    }

    pub fn create_window(&self) -> Result<Window> {
        // SAFETY: The context proves this call is on the initialized UI thread.
        let raw = unsafe { ffi::zpd_winui3_window_create(self.raw) };
        Ok(Window {
            raw: NonNull::new(raw).ok_or(Error(-1))?,
            _main_thread: PhantomData,
        })
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DispatcherQueuePriority {
    Low = -10,
    #[default]
    Normal = 0,
    High = 10,
}

pub struct DispatcherQueue {
    raw: NonNull<ffi::DispatcherQueue>,
}

// SAFETY: The native DispatcherQueue wrapper holds an agile WinRT dispatcher reference.
unsafe impl Send for DispatcherQueue {}
// SAFETY: TryEnqueue may be called concurrently and COM manages the shared reference.
unsafe impl Sync for DispatcherQueue {}

impl Clone for DispatcherQueue {
    fn clone(&self) -> Self {
        // SAFETY: `self.raw` owns a live native dispatcher wrapper.
        let raw = unsafe { ffi::zpd_winui3_dispatcher_clone(self.raw.as_ptr()) };
        Self {
            raw: NonNull::new(raw).expect("failed to clone DispatcherQueue"),
        }
    }
}

impl DispatcherQueue {
    pub fn try_enqueue<F>(&self, priority: DispatcherQueuePriority, task: F) -> bool
    where
        F: FnOnce() + Send + 'static,
    {
        struct Task<F>(Option<F>);

        unsafe extern "C" fn invoke<F: FnOnce()>(data: *const c_void) {
            abort_on_panic(|| {
                // SAFETY: The allocation is kept alive by native code during this call.
                let task = unsafe { &mut *data.cast_mut().cast::<Task<F>>() };
                task.0.take().expect("dispatcher task ran twice")();
            });
        }

        unsafe extern "C" fn release<F>(data: *const c_void) {
            if !data.is_null() {
                // SAFETY: Native code consumes the transferred Box exactly once.
                unsafe { drop(Box::from_raw(data.cast_mut().cast::<Task<F>>())) };
            }
        }

        let task = Box::into_raw(Box::new(Task(Some(task))));
        // SAFETY: Native code takes ownership of `task`, including when enqueue fails.
        unsafe {
            ffi::zpd_winui3_dispatcher_try_enqueue(
                self.raw.as_ptr(),
                priority as i32,
                task.cast(),
                invoke::<F>,
                release::<F>,
            )
        }
    }
}

impl Drop for DispatcherQueue {
    fn drop(&mut self) {
        // SAFETY: This releases one independently owned native wrapper.
        unsafe { ffi::zpd_winui3_dispatcher_release(self.raw.as_ptr()) };
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SystemBackdrop {
    #[default]
    None = 0,
    Mica = 1,
    DesktopAcrylic = 2,
}

pub struct Window {
    raw: NonNull<ffi::Window>,
    _main_thread: PhantomData<Rc<()>>,
}

impl Window {
    pub fn set_title(&self, title: &str) -> Result<()> {
        // SAFETY: The window is live and the string is borrowed for this call.
        check(unsafe {
            ffi::zpd_winui3_window_set_title(self.raw.as_ptr(), ffi::StringRef::new(title))
        })
    }

    pub fn resize(&self, width: i32, height: i32) -> Result<()> {
        // SAFETY: The window is live on the UI thread.
        check(unsafe { ffi::zpd_winui3_window_resize(self.raw.as_ptr(), width, height) })
    }

    pub fn activate(&self) -> Result<()> {
        // SAFETY: The window is live on the UI thread.
        check(unsafe { ffi::zpd_winui3_window_activate(self.raw.as_ptr()) })
    }

    pub fn close(&self) -> Result<()> {
        // SAFETY: The window is live on the UI thread.
        check(unsafe { ffi::zpd_winui3_window_close(self.raw.as_ptr()) })
    }

    pub fn set_content(&self, content: Option<&impl AsElement>) -> Result<()> {
        let raw = content.map_or(std::ptr::null(), |value| value.as_element().as_ptr());
        // SAFETY: Both handles are live and native code retains the XAML element.
        check(unsafe { ffi::zpd_winui3_window_set_content(self.raw.as_ptr(), raw) })
    }

    pub fn set_extends_content_into_title_bar(&self, enabled: bool) -> Result<()> {
        // SAFETY: The window is live on the UI thread.
        check(unsafe {
            ffi::zpd_winui3_window_extend_content_into_title_bar(self.raw.as_ptr(), enabled)
        })
    }

    pub fn set_title_bar(&self, element: Option<&impl AsElement>) -> Result<()> {
        let raw = element.map_or(std::ptr::null(), |value| value.as_element().as_ptr());
        // SAFETY: Both handles are live and WinUI retains the title-bar element.
        check(unsafe { ffi::zpd_winui3_window_set_title_bar(self.raw.as_ptr(), raw) })
    }

    pub fn set_system_backdrop(&self, backdrop: SystemBackdrop) -> Result<()> {
        // SAFETY: The window is live on the UI thread.
        check(unsafe { ffi::zpd_winui3_window_set_backdrop(self.raw.as_ptr(), backdrop as i32) })
    }

    pub fn set_menu_bar<F>(&self, menu_bar: &MenuBar, callback: F) -> Result<()>
    where
        F: FnMut(&str) + 'static,
    {
        let native = NativeMenuBar::new(menu_bar);
        let callback = Rc::into_raw(Rc::new(RefCell::new(callback)));
        // SAFETY: Native code owns the transferred Rc and only borrows menu data for this call.
        check(unsafe {
            ffi::zpd_winui3_window_set_menu_bar(
                self.raw.as_ptr(),
                native.raw.as_ptr(),
                native.raw.len(),
                callback.cast(),
                invoke_string::<F>,
                release_callback::<F>,
            )
        })
    }

    pub fn clear_menu_bar(&self) -> Result<()> {
        // SAFETY: The window is live and clears its owned callback state.
        check(unsafe { ffi::zpd_winui3_window_clear_menu_bar(self.raw.as_ptr()) })
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        // SAFETY: This owned wrapper is released exactly once on the UI thread.
        unsafe { ffi::zpd_winui3_window_release(self.raw.as_ptr()) };
    }
}

pub struct ElementRef<'element> {
    raw: NonNull<ffi::Element>,
    _element: PhantomData<&'element ()>,
    _main_thread: PhantomData<Rc<()>>,
}

impl ElementRef<'_> {
    fn as_ptr(&self) -> *const ffi::Element {
        self.raw.as_ptr()
    }
}

struct OwnedElement {
    raw: NonNull<ffi::Element>,
    _main_thread: PhantomData<Rc<()>>,
}

impl OwnedElement {
    unsafe fn from_raw(raw: *mut ffi::Element) -> Result<Self> {
        Ok(Self {
            raw: NonNull::new(raw).ok_or(Error(-1))?,
            _main_thread: PhantomData,
        })
    }

    fn as_element(&self) -> ElementRef<'_> {
        ElementRef {
            raw: self.raw,
            _element: PhantomData,
            _main_thread: PhantomData,
        }
    }
}

impl Drop for OwnedElement {
    fn drop(&mut self) {
        // SAFETY: This owned native element wrapper is released exactly once.
        unsafe { ffi::zpd_winui3_element_release(self.raw.as_ptr()) };
    }
}

pub trait AsElement {
    fn as_element(&self) -> ElementRef<'_>;

    fn set_margin(&self, value: Thickness) -> Result<()> {
        let raw = ffi::Thickness {
            left: value.left,
            top: value.top,
            right: value.right,
            bottom: value.bottom,
        };
        // SAFETY: The element is live and the call is synchronous.
        check(unsafe { ffi::zpd_winui3_element_set_margin(self.as_element().raw.as_ptr(), raw) })
    }

    fn set_width(&self, value: f64) -> Result<()> {
        // SAFETY: The element is live on the UI thread.
        check(unsafe { ffi::zpd_winui3_element_set_width(self.as_element().raw.as_ptr(), value) })
    }

    fn set_height(&self, value: f64) -> Result<()> {
        // SAFETY: The element is live on the UI thread.
        check(unsafe { ffi::zpd_winui3_element_set_height(self.as_element().raw.as_ptr(), value) })
    }

    fn set_horizontal_alignment(&self, value: HorizontalAlignment) -> Result<()> {
        // SAFETY: The element is live on the UI thread.
        check(unsafe {
            ffi::zpd_winui3_element_set_horizontal_alignment(
                self.as_element().raw.as_ptr(),
                value as i32,
            )
        })
    }

    fn set_vertical_alignment(&self, value: VerticalAlignment) -> Result<()> {
        // SAFETY: The element is live on the UI thread.
        check(unsafe {
            ffi::zpd_winui3_element_set_vertical_alignment(
                self.as_element().raw.as_ptr(),
                value as i32,
            )
        })
    }
}

macro_rules! element_type {
    ($name:ident) => {
        impl AsElement for $name {
            fn as_element(&self) -> ElementRef<'_> {
                self.element.as_element()
            }
        }
    };
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Thickness {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

impl Thickness {
    pub const fn uniform(value: f64) -> Self {
        Self {
            left: value,
            top: value,
            right: value,
            bottom: value,
        }
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HorizontalAlignment {
    Left = 0,
    Center = 1,
    Right = 2,
    #[default]
    Stretch = 3,
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum VerticalAlignment {
    Top = 0,
    Center = 1,
    Bottom = 2,
    #[default]
    Stretch = 3,
}

pub struct Button {
    element: OwnedElement,
}
element_type!(Button);

impl Button {
    pub fn new(content: &str) -> Result<Self> {
        // SAFETY: Native creation borrows the UTF-8 string synchronously.
        let raw = unsafe { ffi::zpd_winui3_button_create(ffi::StringRef::new(content)) };
        // SAFETY: A non-null result is a newly owned element wrapper.
        Ok(Self {
            element: unsafe { OwnedElement::from_raw(raw)? },
        })
    }

    pub fn set_content(&self, content: &str) -> Result<()> {
        // SAFETY: The handle is a live Button and the string borrow covers the call.
        check(unsafe {
            ffi::zpd_winui3_button_set_title(
                self.element.raw.as_ptr(),
                ffi::StringRef::new(content),
            )
        })
    }

    pub fn set_is_enabled(&self, enabled: bool) -> Result<()> {
        // SAFETY: The handle is a live Button.
        check(unsafe { ffi::zpd_winui3_button_set_enabled(self.element.raw.as_ptr(), enabled) })
    }

    pub fn set_click<F: FnMut() + 'static>(&self, callback: F) -> Result<()> {
        let callback = Rc::into_raw(Rc::new(RefCell::new(callback)));
        // SAFETY: Native code takes ownership of one Rc strong reference.
        check(unsafe {
            ffi::zpd_winui3_button_set_click_handler(
                self.element.raw.as_ptr(),
                callback.cast(),
                invoke_unit::<F>,
                release_callback::<F>,
            )
        })
    }

    pub fn clear_click(&self) -> Result<()> {
        // SAFETY: The handle is a live Button.
        check(unsafe { ffi::zpd_winui3_button_clear_click_handler(self.element.raw.as_ptr()) })
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextAlignment {
    #[default]
    Center = 0,
    Left = 1,
    Right = 2,
    Justify = 3,
    DetectFromContent = 4,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TextWrapping {
    #[default]
    NoWrap,
    Wrap,
}

pub struct TextBlock {
    element: OwnedElement,
}
element_type!(TextBlock);

impl TextBlock {
    pub fn new(text: &str) -> Result<Self> {
        // SAFETY: Native creation borrows the UTF-8 string synchronously.
        let raw = unsafe { ffi::zpd_winui3_text_create(ffi::StringRef::new(text)) };
        // SAFETY: A non-null result is a newly owned element wrapper.
        Ok(Self {
            element: unsafe { OwnedElement::from_raw(raw)? },
        })
    }
    pub fn set_text(&self, text: &str) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_text_set_text(self.element.raw.as_ptr(), ffi::StringRef::new(text))
        })
    }
    pub fn set_text_wrapping(&self, value: TextWrapping) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_text_set_wrapping(
                self.element.raw.as_ptr(),
                value == TextWrapping::Wrap,
            )
        })
    }
    pub fn set_text_alignment(&self, value: TextAlignment) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_text_set_alignment(self.element.raw.as_ptr(), value as i32)
        })
    }
}

pub struct TextBox {
    element: OwnedElement,
}
element_type!(TextBox);

impl TextBox {
    pub fn new(text: &str) -> Result<Self> {
        // SAFETY: Native creation borrows the UTF-8 string synchronously.
        let raw = unsafe { ffi::zpd_winui3_text_field_create(ffi::StringRef::new(text)) };
        // SAFETY: A non-null result is a newly owned element wrapper.
        Ok(Self {
            element: unsafe { OwnedElement::from_raw(raw)? },
        })
    }
    pub fn set_text(&self, text: &str) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_text_field_set_value(
                self.element.raw.as_ptr(),
                ffi::StringRef::new(text),
            )
        })
    }
    pub fn text(&self) -> Result<String> {
        unsafe extern "C" fn receive(data: *const c_void, value: ffi::StringRef) {
            // SAFETY: Native code provides a valid UTF-8 buffer for this synchronous callback.
            let bytes = unsafe { std::slice::from_raw_parts(value.data, value.length) };
            // SAFETY: Native strings originate from WinRT hstrings and are encoded as UTF-8.
            unsafe {
                *data.cast_mut().cast::<String>() = String::from_utf8_unchecked(bytes.to_vec())
            };
        }
        let mut result = String::new();
        check(unsafe {
            ffi::zpd_winui3_text_field_get_value(
                self.element.raw.as_ptr(),
                std::ptr::from_mut(&mut result).cast(),
                receive,
            )
        })?;
        Ok(result)
    }
    pub fn set_placeholder_text(&self, text: &str) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_text_field_set_placeholder(
                self.element.raw.as_ptr(),
                ffi::StringRef::new(text),
            )
        })
    }
    pub fn set_is_read_only(&self, value: bool) -> Result<()> {
        check(unsafe { ffi::zpd_winui3_text_field_set_read_only(self.element.raw.as_ptr(), value) })
    }
    pub fn set_text_changed<F: FnMut(String) + 'static>(&self, callback: F) -> Result<()> {
        let callback = Rc::into_raw(Rc::new(RefCell::new(callback)));
        check(unsafe {
            ffi::zpd_winui3_text_field_set_change_handler(
                self.element.raw.as_ptr(),
                callback.cast(),
                invoke_owned_string::<F>,
                release_callback::<F>,
            )
        })
    }
    pub fn clear_text_changed(&self) -> Result<()> {
        check(unsafe { ffi::zpd_winui3_text_field_clear_change_handler(self.element.raw.as_ptr()) })
    }
}

#[repr(i32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Orientation {
    #[default]
    Vertical = 0,
    Horizontal = 1,
}

pub struct StackPanel {
    element: OwnedElement,
}
element_type!(StackPanel);

impl StackPanel {
    pub fn new(orientation: Orientation, spacing: f64) -> Result<Self> {
        let raw = unsafe { ffi::zpd_winui3_stack_panel_create(orientation as i32, spacing) };
        Ok(Self {
            element: unsafe { OwnedElement::from_raw(raw)? },
        })
    }
    pub fn append(&self, child: &impl AsElement) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_panel_append(self.element.raw.as_ptr(), child.as_element().as_ptr())
        })
    }
    pub fn clear(&self) -> Result<()> {
        check(unsafe { ffi::zpd_winui3_panel_clear(self.element.raw.as_ptr()) })
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GridLength {
    Auto,
    Pixel(f64),
    Star(f64),
}

impl GridLength {
    fn native(self) -> ffi::GridLength {
        match self {
            Self::Auto => ffi::GridLength {
                kind: 0,
                value: 0.0,
            },
            Self::Pixel(value) => ffi::GridLength { kind: 1, value },
            Self::Star(value) => ffi::GridLength { kind: 2, value },
        }
    }
}

pub struct Grid {
    element: OwnedElement,
}
element_type!(Grid);

impl Grid {
    pub fn new() -> Result<Self> {
        let raw = unsafe { ffi::zpd_winui3_grid_create() };
        Ok(Self {
            element: unsafe { OwnedElement::from_raw(raw)? },
        })
    }
    pub fn set_row_definitions(&self, values: &[GridLength]) -> Result<()> {
        let values: Vec<_> = values.iter().copied().map(GridLength::native).collect();
        check(unsafe {
            ffi::zpd_winui3_grid_set_rows(self.element.raw.as_ptr(), values.as_ptr(), values.len())
        })
    }
    pub fn set_column_definitions(&self, values: &[GridLength]) -> Result<()> {
        let values: Vec<_> = values.iter().copied().map(GridLength::native).collect();
        check(unsafe {
            ffi::zpd_winui3_grid_set_columns(
                self.element.raw.as_ptr(),
                values.as_ptr(),
                values.len(),
            )
        })
    }
    pub fn add(
        &self,
        child: &impl AsElement,
        row: i32,
        column: i32,
        row_span: i32,
        column_span: i32,
    ) -> Result<()> {
        check(unsafe {
            ffi::zpd_winui3_grid_add(
                self.element.raw.as_ptr(),
                child.as_element().as_ptr(),
                row,
                column,
                row_span,
                column_span,
            )
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MenuBar {
    pub items: Vec<MenuBarItem>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuBarItem {
    pub title: String,
    pub items: Vec<MenuFlyoutItemBase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuFlyoutItemBase {
    Item(MenuFlyoutItem),
    SubItem(MenuFlyoutSubItem),
    Separator,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuFlyoutItem {
    pub id: String,
    pub text: String,
    pub is_enabled: bool,
    pub keyboard_accelerator: Option<KeyboardAccelerator>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuFlyoutSubItem {
    pub text: String,
    pub is_enabled: bool,
    pub items: Vec<MenuFlyoutItemBase>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyboardAccelerator {
    pub key: String,
    pub modifiers: VirtualKeyModifiers,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct VirtualKeyModifiers(pub u32);

impl VirtualKeyModifiers {
    pub const NONE: Self = Self(0);
    pub const CONTROL: Self = Self(1);
    pub const MENU: Self = Self(2);
    pub const SHIFT: Self = Self(4);
    pub const WINDOWS: Self = Self(8);
}

impl std::ops::BitOr for VirtualKeyModifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

struct NativeMenuItem {
    raw: ffi::MenuFlyoutItem,
    _children: Vec<NativeMenuItem>,
    _child_raw: Vec<ffi::MenuFlyoutItem>,
}

impl NativeMenuItem {
    fn new(source: &MenuFlyoutItemBase) -> Self {
        let (kind, id, title, enabled, accelerator, sources): (
            i32,
            &str,
            &str,
            bool,
            Option<&KeyboardAccelerator>,
            &[_],
        ) = match source {
            MenuFlyoutItemBase::Item(item) => (
                0,
                &item.id,
                &item.text,
                item.is_enabled,
                item.keyboard_accelerator.as_ref(),
                &[],
            ),
            MenuFlyoutItemBase::SubItem(item) => {
                (1, "", &item.text, item.is_enabled, None, &item.items)
            }
            MenuFlyoutItemBase::Separator => (2, "", "", true, None, &[]),
        };
        let children: Vec<_> = sources.iter().map(Self::new).collect();
        let child_raw: Vec<_> = children.iter().map(|child| child.raw).collect();
        let (key, modifiers) =
            accelerator.map_or(("", 0), |value| (value.key.as_str(), value.modifiers.0));
        let raw = ffi::MenuFlyoutItem {
            kind,
            id: ffi::StringRef::new(id),
            title: ffi::StringRef::new(title),
            key: ffi::StringRef::new(key),
            modifiers,
            enabled,
            children: child_raw.as_ptr(),
            children_length: child_raw.len(),
        };
        Self {
            raw,
            _children: children,
            _child_raw: child_raw,
        }
    }
}

struct NativeMenuBar {
    _items: Vec<Vec<NativeMenuItem>>,
    _item_raw: Vec<Vec<ffi::MenuFlyoutItem>>,
    raw: Vec<ffi::MenuBarItem>,
}

impl NativeMenuBar {
    fn new(source: &MenuBar) -> Self {
        let items: Vec<Vec<_>> = source
            .items
            .iter()
            .map(|menu| menu.items.iter().map(NativeMenuItem::new).collect())
            .collect();
        let item_raw: Vec<Vec<_>> = items
            .iter()
            .map(|items| items.iter().map(|item| item.raw).collect())
            .collect();
        let raw = source
            .items
            .iter()
            .zip(&item_raw)
            .map(|(menu, items)| ffi::MenuBarItem {
                title: ffi::StringRef::new(&menu.title),
                items: items.as_ptr(),
                items_length: items.len(),
            })
            .collect();
        Self {
            _items: items,
            _item_raw: item_raw,
            raw,
        }
    }
}

unsafe fn clone_callback<F>(data: *const c_void) -> Rc<RefCell<F>> {
    let pointer = data.cast::<RefCell<F>>();
    // SAFETY: Native code owns one strong reference while the callback is installed.
    unsafe { Rc::increment_strong_count(pointer) };
    // SAFETY: The increment above created the returned strong reference.
    unsafe { Rc::from_raw(pointer) }
}

unsafe extern "C" fn invoke_unit<F: FnMut()>(data: *const c_void) {
    abort_on_panic(|| {
        // SAFETY: This callback was installed with an Rc-backed state of type F.
        let callback = unsafe { clone_callback::<F>(data) };
        callback
            .try_borrow_mut()
            .unwrap_or_else(|_| std::process::abort())();
    });
}

unsafe extern "C" fn invoke_string<F: FnMut(&str)>(data: *const c_void, value: ffi::StringRef) {
    abort_on_panic(|| {
        // SAFETY: Native code supplies a valid UTF-8 buffer for this synchronous call.
        let value = unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(value.data, value.length))
        };
        // SAFETY: This callback was installed with an Rc-backed state of type F.
        let callback = unsafe { clone_callback::<F>(data) };
        callback
            .try_borrow_mut()
            .unwrap_or_else(|_| std::process::abort())(value);
    });
}

unsafe extern "C" fn invoke_owned_string<F: FnMut(String)>(
    data: *const c_void,
    value: ffi::StringRef,
) {
    abort_on_panic(|| {
        // SAFETY: Native code supplies a valid UTF-8 buffer for this synchronous call.
        let bytes = unsafe { std::slice::from_raw_parts(value.data, value.length) };
        // SAFETY: Native strings originate from WinRT hstrings and are UTF-8 encoded.
        let value = unsafe { String::from_utf8_unchecked(bytes.to_vec()) };
        // SAFETY: This callback was installed with an Rc-backed state of type F.
        let callback = unsafe { clone_callback::<F>(data) };
        callback
            .try_borrow_mut()
            .unwrap_or_else(|_| std::process::abort())(value);
    });
}

unsafe extern "C" fn release_callback<F>(data: *const c_void) {
    if !data.is_null() {
        // SAFETY: This consumes the native owner's one Rc strong reference.
        unsafe { drop(Rc::from_raw(data.cast::<RefCell<F>>())) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn grid_lengths_preserve_winui_kinds_and_values() {
        // Verifies Rust GridLength variants use the native Auto, Pixel, and Star wire contract.
        assert_eq!(GridLength::Auto.native().kind, 0);
        assert_eq!(GridLength::Pixel(12.0).native().value, 12.0);
        assert_eq!(GridLength::Star(2.0).native().kind, 2);
    }

    #[test]
    fn nested_menu_keeps_child_storage_alive() {
        // Verifies recursive menu pointers refer to storage owned for the complete FFI call.
        let menu = MenuBar {
            items: vec![MenuBarItem {
                title: "File".into(),
                items: vec![MenuFlyoutItemBase::SubItem(MenuFlyoutSubItem {
                    text: "Open recent".into(),
                    is_enabled: true,
                    items: vec![MenuFlyoutItemBase::Item(MenuFlyoutItem {
                        id: "open".into(),
                        text: "Open".into(),
                        is_enabled: true,
                        keyboard_accelerator: None,
                    })],
                })],
            }],
        };
        let native = NativeMenuBar::new(&menu);
        assert_eq!(native.raw.len(), 1);
        assert_eq!(native._item_raw[0][0].children_length, 1);
        assert!(!native._item_raw[0][0].children.is_null());
    }

    #[test]
    fn modifier_flags_compose_without_losing_bits() {
        // Verifies WinUI keyboard modifier flags can be combined for accelerators.
        let value = VirtualKeyModifiers::CONTROL | VirtualKeyModifiers::SHIFT;
        assert_eq!(value.0, 5);
    }

    #[test]
    fn native_dispatcher_releases_a_rejected_task() {
        // Verifies the C++ boundary releases task ownership when no dispatcher can enqueue it.
        struct Probe(Arc<AtomicUsize>);
        impl Drop for Probe {
            fn drop(&mut self) {
                self.0.fetch_add(1, Ordering::Relaxed);
            }
        }
        unsafe extern "C" fn invoke(_: *const c_void) {}
        unsafe extern "C" fn release(data: *const c_void) {
            // SAFETY: The test transfers exactly one Box to the native rejection path.
            unsafe { drop(Box::from_raw(data.cast_mut().cast::<Probe>())) };
        }

        let drops = Arc::new(AtomicUsize::new(0));
        let probe = Box::into_raw(Box::new(Probe(drops.clone())));
        // SAFETY: A null dispatcher is an intentional rejected-enqueue test case.
        let accepted = unsafe {
            ffi::zpd_winui3_dispatcher_try_enqueue(
                std::ptr::null(),
                0,
                probe.cast(),
                invoke,
                release,
            )
        };
        assert!(!accepted);
        assert_eq!(drops.load(Ordering::Relaxed), 1);
    }
}
