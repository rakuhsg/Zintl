use std::cell::RefCell;

use crate::native::{self, Strong};
use crate::runloop::{Application, ApplicationDelegate};

use super::callback;
use super::view::{AsView, OwnedView, ViewError, ViewRef};

pub struct Button {
    view: OwnedView,
    target: RefCell<Option<Strong>>,
}
impl Button {
    pub fn with_title<D: ApplicationDelegate>(
        application: &Application<D>,
        title: &str,
    ) -> Result<Self, ViewError> {
        let title = native::nsstring(title);
        // SAFETY: NSButton's title constructor returns an autoreleased live object, retained here.
        let native = unsafe {
            let value = native::send_id(
                native::send_id(native::class(b"NSButton\0"), native::sel(b"alloc\0")),
                native::sel(b"init\0"),
            );
            let value = Strong::from_retained(value).ok_or(ViewError::NativeCreationFailed)?;
            native::send_void_id(value.as_ptr(), native::sel(b"setTitle:\0"), title.as_ptr());
            value
        };
        Ok(Self {
            view: OwnedView::new(application, native),
            target: RefCell::new(None),
        })
    }
    pub fn set_title(&self, title: &str) -> Result<(), ViewError> {
        let title = native::nsstring(title);
        self.view.as_view().with(|button| unsafe {
            native::send_void_id(button, native::sel(b"setTitle:\0"), title.as_ptr())
        })
    }
    pub fn set_action<F>(&self, mut action: F) -> Result<(), ViewError>
    where
        F: FnMut() + 'static,
    {
        let target = callback::target(move |_| action());
        self.view.as_view().with(|button| unsafe {
            native::send_void_id(button, native::sel(b"setTarget:\0"), target.as_ptr());
            native::send_void_id(
                button,
                native::sel(b"setAction:\0"),
                native::sel(b"invoke:\0"),
            );
        })?;
        *self.target.borrow_mut() = Some(target);
        Ok(())
    }
    pub fn clear_action(&self) -> Result<(), ViewError> {
        self.view.as_view().with(|button| unsafe {
            native::send_void_id(button, native::sel(b"setTarget:\0"), native::NIL);
            native::send_void_id(button, native::sel(b"setAction:\0"), native::NIL);
        })?;
        self.target.borrow_mut().take();
        Ok(())
    }
}
impl AsView for Button {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
