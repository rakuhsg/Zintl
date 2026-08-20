use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::ffi;
use crate::runloop::{Application, ApplicationDelegate};
use crate::string::{NativeOptionalString, NativeString, receive_native_string};

use super::view::{AsView, OwnedView, ViewError, ViewRef};

struct ChangeState<F> {
    callback: RefCell<F>,
}

unsafe extern "C" fn perform_change<F: FnMut(String)>(
    user_data: *const c_void,
    value: NativeString,
) {
    if user_data.is_null() {
        return;
    }
    // SAFETY: Native code owns this Rc strong reference until `release_change`.
    let state = unsafe { Rc::from_raw(user_data.cast::<ChangeState<F>>()) };
    // SAFETY: The native callback owns the UTF-8 buffer for this synchronous call.
    let value = unsafe { value.to_string() };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut callback) = state.callback.try_borrow_mut() else {
            std::process::abort();
        };
        callback(value);
    }));
    let _ = Rc::into_raw(state);
    if result.is_err() {
        std::process::abort();
    }
}

unsafe extern "C" fn release_change<F>(user_data: *const c_void) {
    if !user_data.is_null() {
        // SAFETY: This consumes the Rc strong reference transferred to native code.
        unsafe { drop(Rc::from_raw(user_data.cast::<ChangeState<F>>())) };
    }
}

/// Owns a strong reference to an AppKit `NSTextField`.
pub struct TextField {
    view: OwnedView,
}

impl TextField {
    pub fn with_string<D: ApplicationDelegate>(
        _application: &Application<D>,
        value: &str,
    ) -> Result<Self, ViewError> {
        Self::create(value, false)
    }

    pub fn label_with_string<D: ApplicationDelegate>(
        _application: &Application<D>,
        value: &str,
    ) -> Result<Self, ViewError> {
        Self::create(value, true)
    }

    fn create(value: &str, label: bool) -> Result<Self, ViewError> {
        // SAFETY: The public constructors require a main-thread-bound
        // Application and the returned text field has a +1 retain count.
        let view = unsafe {
            OwnedView::from_raw(ffi::zintlappkit_create_text_field(
                NativeString::from_str(value),
                label,
            ))?
        };
        Ok(Self { view })
    }

    pub fn set_string_value(&self, value: &str) {
        // SAFETY: `self` owns a live NSTextField on the AppKit main thread.
        unsafe {
            ffi::zintlappkit_text_field_set_string_value(
                self.view.as_view().as_ptr(),
                NativeString::from_str(value),
            );
        }
    }

    pub fn string_value(&self) -> String {
        receive_native_string(|user_data, callback| {
            // SAFETY: The text field is live on the AppKit main thread. The
            // native function invokes `callback` synchronously.
            unsafe {
                ffi::zintlappkit_text_field_get_string_value(
                    self.view.as_view().as_ptr(),
                    user_data,
                    callback,
                );
            }
        })
    }

    pub fn set_placeholder_string(&self, value: Option<&str>) {
        // SAFETY: The optional native string borrows `value` for this
        // synchronous main-thread call.
        unsafe {
            ffi::zintlappkit_text_field_set_placeholder_string(
                self.view.as_view().as_ptr(),
                NativeOptionalString::from_option(value),
            );
        }
    }

    pub fn set_editable(&self, editable: bool) {
        // SAFETY: `self` owns a live NSTextField on the AppKit main thread.
        unsafe {
            ffi::zintlappkit_text_field_set_editable(self.view.as_view().as_ptr(), editable);
        }
    }

    pub fn set_selectable(&self, selectable: bool) {
        // SAFETY: `self` owns a live NSTextField on the AppKit main thread.
        unsafe {
            ffi::zintlappkit_text_field_set_selectable(self.view.as_view().as_ptr(), selectable);
        }
    }

    pub fn set_change_handler<F>(&self, callback: F)
    where
        F: FnMut(String) + 'static,
    {
        let state = Rc::new(ChangeState {
            callback: RefCell::new(callback),
        });
        let user_data = Rc::into_raw(state);
        // SAFETY: Native code owns the transferred Rc reference until the
        // handler is cleared or the text field is destroyed.
        unsafe {
            ffi::zintlappkit_text_field_set_change_handler(
                self.view.as_view().as_ptr(),
                user_data.cast(),
                perform_change::<F>,
                release_change::<F>,
            );
        }
    }

    pub fn clear_change_handler(&self) {
        // SAFETY: `self` owns a live editable NSTextField on the main thread.
        unsafe {
            ffi::zintlappkit_text_field_clear_change_handler(self.view.as_view().as_ptr());
        }
    }
}

impl AsView for TextField {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
