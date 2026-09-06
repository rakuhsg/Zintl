use crate::actor::WindowEventKind;
use crate::native;
use crate::runloop::{Application, ApplicationDelegate};
use zpd_objc::Strong;

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
            let value = zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSButton"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("init"), () => zpd_objc::Id);
            let value = Strong::from_retained(value).ok_or(ViewError::NativeCreationFailed)?;
            zpd_objc::msg_send!(value.as_ptr(), zpd_objc::sel!("setTitle:"), ((title.as_ptr()): zpd_objc::Id) => ());
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
                zpd_objc::msg_send!(button, zpd_objc::sel!("setTarget:"), ((target): zpd_objc::Id) => ());
                zpd_objc::msg_send!(button, zpd_objc::sel!("setAction:"), ((zpd_objc::sel!("invoke:")): zpd_objc::Id) => ());
            })
        })??;
        application
            .tree()
            .add_teardown(view.actor(), |button| unsafe {
                zpd_objc::msg_send!(button, zpd_objc::sel!("setTarget:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
                zpd_objc::msg_send!(button, zpd_objc::sel!("setAction:"), ((zpd_objc::NIL): zpd_objc::Id) => ());
            })
            .map_err(ViewError::from)?;
        Ok(Self { view })
    }
    pub fn set_title(&self, title: &str) -> Result<(), ViewError> {
        let title = native::nsstring(title);
        self.view.as_view().with(|button| unsafe {
            zpd_objc::msg_send!(button, zpd_objc::sel!("setTitle:"), ((title.as_ptr()): zpd_objc::Id) => ())
        })
    }
}
impl AsView for Button {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
