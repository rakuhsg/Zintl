use std::cell::{Cell, RefCell};
use std::ffi::{CStr, CString, c_char, c_void};
use std::marker::PhantomData;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;
use std::rc::Rc;

#[cfg(feature = "wgpu")]
use crate::geometry::PhysicalSize;
use crate::geometry::Rect;
use crate::{ffi, runloop::Application};

use super::CommandSet;

/// Receives native window lifecycle notifications on the AppKit main thread.
pub trait WindowDelegate: 'static {
    fn did_create(&mut self) {}
    fn will_close(&mut self) {}
    fn did_close(&mut self) {}
}

impl WindowDelegate for () {}

#[derive(Debug)]
pub enum WindowError {
    NativeCreationFailed,
    Closed,
    CommandEncoding(serde_json::Error),
    InvalidCommandJson(std::ffi::NulError),
}

impl std::fmt::Display for WindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NativeCreationFailed => write!(f, "AppKit failed to create a native object"),
            Self::Closed => write!(f, "the window is closed"),
            Self::CommandEncoding(error) => write!(f, "failed to encode commands: {error}"),
            Self::InvalidCommandJson(error) => {
                write!(f, "encoded commands contain an interior NUL byte: {error}")
            }
        }
    }
}

impl std::error::Error for WindowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CommandEncoding(error) => Some(error),
            Self::InvalidCommandJson(error) => Some(error),
            _ => None,
        }
    }
}

struct WindowState<D> {
    delegate: RefCell<D>,
    closed: Cell<bool>,
}

unsafe fn clone_window_state<D>(user_data: *const c_void) -> Option<Rc<WindowState<D>>> {
    let state = user_data.cast::<WindowState<D>>();
    if state.is_null() {
        return None;
    }

    // SAFETY: Native code owns the strong reference transferred during window
    // creation until the did-close callback.
    unsafe { Rc::increment_strong_count(state) };
    // SAFETY: The increment above created the strong reference returned here.
    Some(unsafe { Rc::from_raw(state) })
}

fn abort_on_panic(f: impl FnOnce()) {
    if catch_unwind(AssertUnwindSafe(f)).is_err() {
        std::process::abort();
    }
}

unsafe fn invoke_delegate<D: WindowDelegate>(user_data: *const c_void, f: impl FnOnce(&mut D)) {
    // SAFETY: Window callbacks are only installed with an Rc-backed state.
    let Some(state) = (unsafe { clone_window_state::<D>(user_data) }) else {
        return;
    };

    abort_on_panic(|| {
        let Ok(mut delegate) = state.delegate.try_borrow_mut() else {
            std::process::abort();
        };
        f(&mut delegate);
    });
}

unsafe extern "C" fn did_create<D: WindowDelegate>(user_data: *const c_void) {
    // SAFETY: This is the `WindowState<D>` pointer passed to native creation.
    unsafe { invoke_delegate::<D>(user_data, WindowDelegate::did_create) };
}

unsafe extern "C" fn will_close<D: WindowDelegate>(user_data: *const c_void) {
    // SAFETY: Native code still owns its Rc strong reference on entry.
    let Some(state) = (unsafe { clone_window_state::<D>(user_data) }) else {
        return;
    };
    state.closed.set(true);
    abort_on_panic(|| {
        let Ok(mut delegate) = state.delegate.try_borrow_mut() else {
            std::process::abort();
        };
        delegate.will_close();
    });
}

unsafe extern "C" fn did_close<D: WindowDelegate>(user_data: *const c_void) {
    // SAFETY: Native code still owns its Rc strong reference on entry.
    let Some(state) = (unsafe { clone_window_state::<D>(user_data) }) else {
        return;
    };
    state.closed.set(true);
    abort_on_panic(|| {
        let Ok(mut delegate) = state.delegate.try_borrow_mut() else {
            std::process::abort();
        };
        delegate.did_close();
    });

    // SAFETY: This consumes the native side's strong reference exactly once,
    // when Swift permanently clears the window callback.
    unsafe { drop(Rc::from_raw(user_data.cast::<WindowState<D>>())) };
}

/// Owns an AppKit window and its callback state.
pub struct Window<'application, D: WindowDelegate> {
    raw: NonNull<c_void>,
    state: Rc<WindowState<D>>,
    _application: PhantomData<&'application Application<()>>,
    _main_thread: PhantomData<Rc<()>>,
}

impl<'application, D: WindowDelegate> Window<'application, D> {
    pub(crate) fn new(delegate: D) -> Result<Self, WindowError> {
        let state = Rc::new(WindowState {
            delegate: RefCell::new(delegate),
            closed: Cell::new(false),
        });
        let ffi_state = Rc::into_raw(state.clone());
        let callbacks = ffi::WindowCallback {
            did_create: did_create::<D>,
            will_close: will_close::<D>,
            did_close: did_close::<D>,
        };

        // SAFETY: The Rc-backed callback state has a stable address. Native
        // code owns one strong reference and copies the callback table.
        let raw = unsafe { ffi::zintlappkit_create_window(ffi_state.cast(), &callbacks) };
        let Some(raw) = NonNull::new(raw.cast_mut()) else {
            // SAFETY: Native creation rejected the pointer without retaining
            // it, so reclaim the transferred strong reference.
            unsafe { drop(Rc::from_raw(ffi_state)) };
            return Err(WindowError::NativeCreationFailed);
        };

        Ok(Self {
            raw,
            state,
            _application: PhantomData,
            _main_thread: PhantomData,
        })
    }

    fn ensure_open(&self) -> Result<(), WindowError> {
        if self.state.closed.get() {
            Err(WindowError::Closed)
        } else {
            Ok(())
        }
    }

    pub fn is_closed(&self) -> bool {
        self.state.closed.get()
    }

    pub fn show(&self) -> Result<(), WindowError> {
        self.ensure_open()?;
        // SAFETY: The handle is owned by `self`, and `Window` is restricted to
        // the AppKit main thread.
        unsafe { ffi::zintlappkit_show_window(self.raw.as_ptr()) };
        Ok(())
    }

    pub fn set_bounds(&self, bounds: Rect) -> Result<(), WindowError> {
        self.ensure_open()?;
        // SAFETY: The handle is valid and this method runs on the main thread.
        unsafe { ffi::zintlappkit_window_set_bounds(self.raw.as_ptr(), bounds) };
        Ok(())
    }

    pub fn set_size(&self, width: f64, height: f64) -> Result<(), WindowError> {
        self.ensure_open()?;
        // SAFETY: The handle is valid and this method runs on the main thread.
        unsafe { ffi::zintlappkit_window_set_size(self.raw.as_ptr(), width, height) };
        Ok(())
    }

    pub fn set_position(&self, x: f64, y: f64) -> Result<(), WindowError> {
        self.ensure_open()?;
        // SAFETY: The handle is valid and this method runs on the main thread.
        unsafe { ffi::zintlappkit_window_set_position(self.raw.as_ptr(), x, y) };
        Ok(())
    }

    pub fn set_commands<F>(&self, commands: &CommandSet, callback: F) -> Result<(), WindowError>
    where
        F: FnMut(&str) + 'static,
    {
        self.ensure_open()?;
        let json = serde_json::to_string(commands).map_err(WindowError::CommandEncoding)?;
        let json = CString::new(json).map_err(WindowError::InvalidCommandJson)?;
        let callback_state = Rc::into_raw(Rc::new(CommandCallback {
            callback: RefCell::new(callback),
        }));

        // SAFETY: The JSON buffer lives for the call. Ownership of
        // `callback_state` transfers to Swift, which invokes `release_command`
        // exactly once when replacing, rejecting, or destroying the commands.
        unsafe {
            ffi::zintlappkit_window_set_commands(
                self.raw.as_ptr(),
                json.as_ptr(),
                callback_state.cast(),
                invoke_command::<F>,
                release_command::<F>,
            );
        }
        Ok(())
    }

    #[cfg(feature = "wgpu")]
    pub fn create_wgpu_surface(
        &self,
        rect: Rect,
    ) -> Result<WgpuSurface<'application>, WindowError> {
        self.ensure_open()?;
        // SAFETY: The window handle is valid and called on the main thread.
        let raw = unsafe { ffi::zintlappkit_create_wgpu_surface(self.raw.as_ptr(), rect) };
        let Some(raw) = NonNull::new(raw.cast_mut()) else {
            return Err(WindowError::NativeCreationFailed);
        };
        Ok(WgpuSurface {
            raw,
            _application: PhantomData,
            _main_thread: PhantomData,
        })
    }
}

impl<D: WindowDelegate> Drop for Window<'_, D> {
    fn drop(&mut self) {
        // SAFETY: The native handle is released exactly once, before Rust drops
        // the callback state, and this type can only be dropped on main.
        unsafe { ffi::zintlappkit_destroy_window(self.raw.as_ptr()) };
    }
}

struct CommandCallback<F> {
    callback: RefCell<F>,
}

unsafe fn clone_command<F>(user_data: *const c_void) -> Option<Rc<CommandCallback<F>>> {
    let state = user_data.cast::<CommandCallback<F>>();
    if state.is_null() {
        return None;
    }

    // SAFETY: Swift holds the transferred strong reference until it invokes
    // `release_command`.
    unsafe { Rc::increment_strong_count(state) };
    // SAFETY: The increment above created the strong reference returned here.
    Some(unsafe { Rc::from_raw(state) })
}

unsafe extern "C" fn invoke_command<F: FnMut(&str) + 'static>(
    user_data: *const c_void,
    command_id: *const c_char,
) {
    if user_data.is_null() || command_id.is_null() {
        return;
    }

    abort_on_panic(|| {
        // SAFETY: Command callbacks are installed with an Rc-backed state.
        let Some(state) = (unsafe { clone_command::<F>(user_data) }) else {
            return;
        };
        // SAFETY: Swift provides a NUL-terminated command identifier for this
        // call.
        let command_id = unsafe { CStr::from_ptr(command_id) }.to_string_lossy();
        let Ok(mut callback) = state.callback.try_borrow_mut() else {
            std::process::abort();
        };
        callback(&command_id);
    });
}

unsafe extern "C" fn release_command<F>(user_data: *const c_void) {
    if user_data.is_null() {
        return;
    }

    // SAFETY: This consumes the strong reference transferred by `Rc::into_raw`;
    // Swift calls the release callback exactly once.
    unsafe { drop(Rc::from_raw(user_data.cast::<CommandCallback<F>>())) };
}

#[cfg(feature = "wgpu")]
#[derive(Clone, Copy, Debug)]
pub struct MetalLayer<'surface> {
    raw: NonNull<c_void>,
    _surface: PhantomData<&'surface ()>,
}

#[cfg(feature = "wgpu")]
impl MetalLayer<'_> {
    pub fn as_ptr(self) -> *mut c_void {
        self.raw.as_ptr()
    }
}

#[cfg(feature = "wgpu")]
/// Owns an AppKit view and CAMetalLayer embedded in a window.
pub struct WgpuSurface<'application> {
    raw: NonNull<c_void>,
    _application: PhantomData<&'application Application<()>>,
    _main_thread: PhantomData<Rc<()>>,
}

#[cfg(feature = "wgpu")]
impl WgpuSurface<'_> {
    pub fn set_rect(&self, rect: Rect) {
        // SAFETY: The surface is alive and this type is main-thread-bound.
        unsafe { ffi::zintlappkit_wgpu_surface_set_rect(self.raw.as_ptr(), rect) };
    }

    pub fn drawable_size(&self) -> PhysicalSize {
        let mut width = 0;
        let mut height = 0;
        // SAFETY: The surface is alive and both out-pointers are valid for the
        // duration of this main-thread call.
        unsafe {
            ffi::zintlappkit_wgpu_surface_drawable_size(self.raw.as_ptr(), &mut width, &mut height);
        }
        PhysicalSize::new(width, height)
    }

    pub fn metal_layer(&self) -> Result<MetalLayer<'_>, WindowError> {
        // SAFETY: The surface keeps the CAMetalLayer alive, and this access
        // happens on the main thread.
        let raw = unsafe { ffi::zintlappkit_wgpu_surface_metal_layer(self.raw.as_ptr()) };
        NonNull::new(raw)
            .map(|raw| MetalLayer {
                raw,
                _surface: PhantomData,
            })
            .ok_or(WindowError::NativeCreationFailed)
    }
}

#[cfg(feature = "wgpu")]
impl Drop for WgpuSurface<'_> {
    fn drop(&mut self) {
        // SAFETY: This owned native surface is released exactly once on main.
        unsafe { ffi::zintlappkit_destroy_wgpu_surface(self.raw.as_ptr()) };
    }
}
