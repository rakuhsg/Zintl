use crate::actor::WindowEventKind;
use crate::native;
use crate::runloop::{Application, ApplicationDelegate};
use zpd_objc::Strong;

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
            let object = zpd_objc::msg_send!(zpd_objc::msg_send!(zpd_objc::class!("NSTextField"), zpd_objc::sel!("alloc"), () => zpd_objc::Id), zpd_objc::sel!("init"), () => zpd_objc::Id);
            let object = Strong::from_retained(object).ok_or(ViewError::NativeCreationFailed)?;
            zpd_objc::msg_send!(object.as_ptr(), zpd_objc::sel!("setStringValue:"), ((value.as_ptr()): zpd_objc::Id) => ());
            if label {
                zpd_objc::msg_send!(object.as_ptr(), zpd_objc::sel!("setEditable:"), ((false): bool) => ());
                zpd_objc::msg_send!(object.as_ptr(), zpd_objc::sel!("setSelectable:"), ((false): bool) => ());
                zpd_objc::msg_send!(object.as_ptr(), zpd_objc::sel!("setBordered:"), ((false): bool) => ());
                zpd_objc::msg_send!(object.as_ptr(), zpd_objc::sel!("setDrawsBackground:"), ((false): bool) => ());
            }
            object
        };
        let view = ViewActor::new(application, native);
        application
            .tree()
            .add_teardown(view.actor(), |field| unsafe {
                zpd_objc::msg_send!(field, zpd_objc::sel!("setDelegate:"), ((zpd_objc::NIL): zpd_objc::Id) => ())
            })
            .map_err(ViewError::from)?;
        if !label {
            let actor = view.actor().clone();
            let target = callback::target(move |notification| unsafe {
                let field =
                    zpd_objc::msg_send!(notification, zpd_objc::sel!("object"), () => zpd_objc::Id);
                let value = native::rust_string(
                    zpd_objc::msg_send!(field, zpd_objc::sel!("stringValue"), () => zpd_objc::Id),
                );
                if let Some(tree) = actor.tree_handle() {
                    tree.emit(&actor, WindowEventKind::TextChanged { value });
                }
            });
            let tree = view.actor().tree_handle().ok_or(ViewError::Closed)?;
            let target = tree
                .replace_owned(view.actor(), "delegate", target)
                .map_err(ViewError::from)?;
            tree.add_teardown(&target, |target| unsafe { callback::release(target) })
                .map_err(ViewError::from)?;
            view.as_view().with(|field| {
                target.with(|target| unsafe {
                    zpd_objc::msg_send!(field, zpd_objc::sel!("setDelegate:"), ((target): zpd_objc::Id) => ())
                })
            })??;
        }
        Ok(Self { view })
    }
    pub fn set_string_value(&self, value: &str) -> Result<(), ViewError> {
        let value = native::nsstring(value);
        self.view.as_view().with(|field| unsafe {
            zpd_objc::msg_send!(field, zpd_objc::sel!("setStringValue:"), ((value.as_ptr()): zpd_objc::Id) => ())
        })
    }
    pub fn string_value(&self) -> Result<String, ViewError> {
        self.view.as_view().with(|field| unsafe {
            native::rust_string(
                zpd_objc::msg_send!(field, zpd_objc::sel!("stringValue"), () => zpd_objc::Id),
            )
        })
    }
    pub fn set_placeholder_string(&self, value: Option<&str>) -> Result<(), ViewError> {
        let value = value.map(native::nsstring);
        self.view.as_view().with(|field| unsafe {
            zpd_objc::msg_send!(field, zpd_objc::sel!("setPlaceholderString:"), ((value.as_ref().map_or(zpd_objc::NIL, Strong::as_ptr)): zpd_objc::Id) => ());
        })
    }
    pub fn set_editable(&self, editable: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            zpd_objc::msg_send!(field, zpd_objc::sel!("setEditable:"), ((editable): bool) => ())
        })
    }
    pub fn set_selectable(&self, selectable: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            zpd_objc::msg_send!(field, zpd_objc::sel!("setSelectable:"), ((selectable): bool) => ())
        })
    }
    pub fn set_bordered(&self, bordered: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            zpd_objc::msg_send!(field, zpd_objc::sel!("setBordered:"), ((bordered): bool) => ())
        })
    }
    pub fn set_draws_background(&self, draws_background: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| unsafe {
            zpd_objc::msg_send!(field, zpd_objc::sel!("setDrawsBackground:"), ((draws_background): bool) => ())
        })
    }
    pub fn set_multiline(&self, multiline: bool) -> Result<(), ViewError> {
        self.view.as_view().with(|field| {
            // SAFETY: The live view is an NSTextField whose cell accepts text layout selectors.
            unsafe {
                let cell = zpd_objc::msg_send!(field, zpd_objc::sel!("cell"), () => zpd_objc::Id);
                let line_break_mode = if multiline {
                    native::NS_LINE_BREAK_BY_WORD_WRAPPING
                } else {
                    native::NS_LINE_BREAK_BY_CLIPPING
                };
                zpd_objc::msg_send!(field, zpd_objc::sel!("setUsesSingleLineMode:"), ((!multiline): bool) => ());
                zpd_objc::msg_send!(field, zpd_objc::sel!("setLineBreakMode:"), ((line_break_mode): u64) => ());
                zpd_objc::msg_send!(cell, zpd_objc::sel!("setScrollable:"), ((!multiline): bool) => ());
                zpd_objc::msg_send!(cell, zpd_objc::sel!("setWraps:"), ((multiline): bool) => ());
                zpd_objc::msg_send!(field, zpd_objc::sel!("setMaximumNumberOfLines:"), ((if multiline { 0 } else { 1 }): isize) => ());
            }
        })
    }
}
impl AsView for TextField {
    fn as_view(&self) -> ViewRef<'_> {
        self.view.as_view()
    }
}
