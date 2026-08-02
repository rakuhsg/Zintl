use std::ffi::CString;

use crate::ffi;
use crate::runloop::{Application, ApplicationDelegate};

use super::view::{AsView, OwnedView, ViewError, ViewRef};

/// Owns a strong reference to an AppKit `NSTextField`.
pub struct TextField<'application> {
    view: OwnedView<'application>,
}

impl<'application> TextField<'application> {
    pub fn with_string<D: ApplicationDelegate>(
        _application: &'application Application<D>,
        value: &str,
    ) -> Result<Self, ViewError> {
        Self::create(value, false)
    }

    pub fn label_with_string<D: ApplicationDelegate>(
        _application: &'application Application<D>,
        value: &str,
    ) -> Result<Self, ViewError> {
        Self::create(value, true)
    }

    fn create(value: &str, label: bool) -> Result<Self, ViewError> {
        let value = CString::new(value).map_err(|_| ViewError::InteriorNul)?;
        // SAFETY: The public constructors require a main-thread-bound
        // Application and the returned text field has a +1 retain count.
        let view = unsafe {
            OwnedView::from_raw(ffi::zintlappkit_create_text_field(value.as_ptr(), label))?
        };
        Ok(Self { view })
    }

    pub fn set_string_value(&self, value: &str) -> Result<(), ViewError> {
        let value = CString::new(value).map_err(|_| ViewError::InteriorNul)?;
        // SAFETY: `self` owns a live NSTextField on the AppKit main thread.
        unsafe {
            ffi::zintlappkit_text_field_set_string_value(
                self.view.as_view().as_ptr(),
                value.as_ptr(),
            );
        }
        Ok(())
    }

    pub fn set_placeholder_string(&self, value: Option<&str>) -> Result<(), ViewError> {
        let value = value
            .map(CString::new)
            .transpose()
            .map_err(|_| ViewError::InteriorNul)?;
        // SAFETY: A null pointer represents `nil`; otherwise the CString lives
        // through this synchronous main-thread call.
        unsafe {
            ffi::zintlappkit_text_field_set_placeholder_string(
                self.view.as_view().as_ptr(),
                value
                    .as_ref()
                    .map_or(std::ptr::null(), |value| value.as_ptr()),
            );
        }
        Ok(())
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
}

impl AsView for TextField<'_> {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
