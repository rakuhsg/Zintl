use crate::native::{self, Strong};
use crate::runloop::{Application, ApplicationDelegate};

use super::callback;
use super::view::{AsView, ViewActor, ViewError, ViewRef};

pub struct TextField {
    view: ViewActor,
}
impl TextField {
    pub fn with_string<D: ApplicationDelegate>(
        application: &Application<D>,
        value: &str,
    ) -> Result<Self, ViewError> {
        Self::create(application, value, false)
    }
    pub fn label_with_string<D: ApplicationDelegate>(
        application: &Application<D>,
        value: &str,
    ) -> Result<Self, ViewError> {
        Self::create(application, value, true)
    }
    fn create<D: ApplicationDelegate>(
        application: &Application<D>,
        value: &str,
        label: bool,
    ) -> Result<Self, ViewError> {
        let value = native::nsstring(value);
        // SAFETY: NSTextField alloc/init produces a retained object.
        let native = unsafe {
            let object = native::send_id(
                native::send_id(native::class(b"NSTextField\0"), native::sel(b"alloc\0")),
                native::sel(b"init\0"),
            );
            let object = Strong::from_retained(object).ok_or(ViewError::NativeCreationFailed)?;
            native::send_void_id(
                object.as_ptr(),
                native::sel(b"setStringValue:\0"),
                value.as_ptr(),
            );
            if label {
                native::send_void_bool(object.as_ptr(), native::sel(b"setEditable:\0"), false);
                native::send_void_bool(object.as_ptr(), native::sel(b"setSelectable:\0"), false);
                native::send_void_bool(object.as_ptr(), native::sel(b"setBordered:\0"), false);
                native::send_void_bool(
                    object.as_ptr(),
                    native::sel(b"setDrawsBackground:\0"),
                    false,
                );
            }
            object
        };
        let view = ViewActor::new(application, native);
        application
            .tree()
            .add_teardown(view.actor(), |field| unsafe {
                native::send_void_id(field, native::sel(b"setDelegate:\0"), native::NIL)
            })
            .map_err(ViewError::from)?;
        Ok(Self { view })
    }
    pub fn set_string_value(&self, value: &str) -> Result<(), ViewError> {
        let value = native::nsstring(value);
        self.view.as_view().with(|field| unsafe {
            native::send_void_id(field, native::sel(b"setStringValue:\0"), value.as_ptr())
        })
    }
    pub fn string_value(&self) -> Result<String, ViewError> {
        self.view.as_view().with(|field| unsafe {
            native::rust_string(native::send_id(field, native::sel(b"stringValue\0")))
        })
    }
    pub fn set_placeholder_string(&self, value: Option<&str>) -> Result<(), ViewError> {
        let value = value.map(native::nsstring);
        self.view.as_view().with(|field| unsafe {
            native::send_void_id(
                field,
                native::sel(b"setPlaceholderString:\0"),
                value.as_ref().map_or(native::NIL, Strong::as_ptr),
            );
        })
    }
    pub fn set_editable(&self, editable: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            native::send_void_bool(field, native::sel(b"setEditable:\0"), editable)
        })
    }
    pub fn set_selectable(&self, selectable: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            native::send_void_bool(field, native::sel(b"setSelectable:\0"), selectable)
        })
    }
    pub fn set_change_handler<F>(&self, mut callback_fn: F) -> Result<(), ViewError>
    where
        F: FnMut(String) + 'static,
    {
        let target = callback::target(move |notification| unsafe {
            let field = native::send_id(notification, native::sel(b"object\0"));
            let value = native::send_id(field, native::sel(b"stringValue\0"));
            callback_fn(native::rust_string(value));
        });
        self.clear_change_handler()?;
        let tree = self.view.actor().tree_handle().ok_or(ViewError::Closed)?;
        let target = tree
            .replace_owned(self.view.actor(), "delegate", target)
            .map_err(ViewError::from)?;
        tree.add_teardown(&target, |target| unsafe { callback::release(target) })
            .map_err(ViewError::from)?;
        self.view.as_view().with(|field| {
            target.with(|target| unsafe {
                native::send_void_id(field, native::sel(b"setDelegate:\0"), target)
            })
        })??;
        Ok(())
    }
    pub fn clear_change_handler(&self) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            native::send_void_id(field, native::sel(b"setDelegate:\0"), native::NIL)
        })?;
        let tree = self.view.actor().tree_handle().ok_or(ViewError::Closed)?;
        tree.clear_owned(self.view.actor(), "delegate")
            .map_err(ViewError::from)?;
        Ok(())
    }
}
impl AsView for TextField {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
