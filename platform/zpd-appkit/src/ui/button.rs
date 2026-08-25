use crate::actor::WindowEventKind;
use crate::native::{self, Strong};
use crate::runloop::{Application, ApplicationDelegate};

use super::callback;
use super::view::{AsView, ViewActor, ViewError, ViewRef};

pub struct Button {
    view: ViewActor,
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
        let view = ViewActor::new(application, native);
        let actor = view.actor().clone();
        let target = callback::target(move |_| {
            if let Some(tree) = actor.tree_handle() {
                tree.emit(&actor, WindowEventKind::ButtonClicked);
            }
        });
        let tree = view.actor().tree_handle().ok_or(ViewError::Closed)?;
        let target = tree
            .replace_owned(view.actor(), "action-target", target)
            .map_err(ViewError::from)?;
        tree.add_teardown(&target, |target| unsafe { callback::release(target) })
            .map_err(ViewError::from)?;
        view.as_view().with(|button| {
            target.with(|target| unsafe {
                native::send_void_id(button, native::sel(b"setTarget:\0"), target);
                native::send_void_id(
                    button,
                    native::sel(b"setAction:\0"),
                    native::sel(b"invoke:\0"),
                );
            })
        })??;
        application
            .tree()
            .add_teardown(view.actor(), |button| unsafe {
                native::send_void_id(button, native::sel(b"setTarget:\0"), native::NIL);
                native::send_void_id(button, native::sel(b"setAction:\0"), native::NIL);
            })
            .map_err(ViewError::from)?;
        Ok(Self { view })
    }
    pub fn set_title(&self, title: &str) -> Result<(), ViewError> {
        let title = native::nsstring(title);
        self.view.as_view().with(|button| unsafe {
            native::send_void_id(button, native::sel(b"setTitle:\0"), title.as_ptr())
        })
    }
}
impl AsView for Button {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
