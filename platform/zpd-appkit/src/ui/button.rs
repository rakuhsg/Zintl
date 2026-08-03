use std::cell::RefCell;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::rc::Rc;

use crate::ffi;
use crate::runloop::{Application, ApplicationDelegate};
use crate::string::NativeString;

use super::view::{AsView, OwnedView, ViewError, ViewRef};

struct ActionState<F> {
    action: RefCell<F>,
}

unsafe extern "C" fn perform_action<F: FnMut()>(user_data: *const c_void) {
    if user_data.is_null() {
        return;
    }
    // SAFETY: Native code owns this Rc strong reference until `release_action`.
    let state = unsafe { Rc::from_raw(user_data.cast::<ActionState<F>>()) };
    let result = catch_unwind(AssertUnwindSafe(|| {
        let Ok(mut action) = state.action.try_borrow_mut() else {
            std::process::abort();
        };
        action();
    }));
    let _ = Rc::into_raw(state);
    if result.is_err() {
        std::process::abort();
    }
}

unsafe extern "C" fn release_action<F>(user_data: *const c_void) {
    if !user_data.is_null() {
        // SAFETY: This consumes the one Rc strong reference transferred to
        // the native action target.
        unsafe { drop(Rc::from_raw(user_data.cast::<ActionState<F>>())) };
    }
}

/// Owns a strong reference to an AppKit `NSButton`.
pub struct Button {
    view: OwnedView,
}

impl Button {
    pub fn with_title<D: ApplicationDelegate>(
        _application: &Application<D>,
        title: &str,
    ) -> Result<Self, ViewError> {
        // SAFETY: `Application` proves main-thread access and the returned
        // native button has a +1 retain count.
        let view = unsafe {
            OwnedView::from_raw(ffi::zintlappkit_create_button(NativeString::from_str(
                title,
            )))?
        };
        Ok(Self { view })
    }

    pub fn set_title(&self, title: &str) {
        // SAFETY: `self` owns a live NSButton on the AppKit main thread.
        unsafe {
            ffi::zintlappkit_button_set_title(
                self.view.as_view().as_ptr(),
                NativeString::from_str(title),
            )
        };
    }

    pub fn set_action<F>(&self, action: F)
    where
        F: FnMut() + 'static,
    {
        let state = Rc::new(ActionState {
            action: RefCell::new(action),
        });
        let user_data = Rc::into_raw(state);
        // SAFETY: Native code takes ownership of the transferred Rc reference
        // and releases it when this action is replaced or the button dies.
        unsafe {
            ffi::zintlappkit_button_set_action(
                self.view.as_view().as_ptr(),
                user_data.cast(),
                perform_action::<F>,
                release_action::<F>,
            );
        }
    }

    pub fn clear_action(&self) {
        // SAFETY: `self` owns a live NSButton on the AppKit main thread.
        unsafe { ffi::zintlappkit_button_clear_action(self.view.as_view().as_ptr()) };
    }
}

impl AsView for Button {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
