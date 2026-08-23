use std::marker::PhantomData;
use std::rc::Rc;

use crate::actor::{ActorError, ActorRef};
use crate::geometry::Rect;
use crate::native::{self, Strong};
use crate::runloop::{Application, ApplicationDelegate};

use super::layout::{Dimension, LayoutAttribute, XAxisAnchor, YAxisAnchor};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewError {
    NativeCreationFailed,
    Closed,
    InvalidHierarchy,
}
impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NativeCreationFailed => "AppKit failed to create a native view",
            Self::Closed => "the native view is closed",
            Self::InvalidHierarchy => "the requested native view hierarchy is invalid",
        })
    }
}
impl std::error::Error for ViewError {}
impl From<ActorError> for ViewError {
    fn from(value: ActorError) -> Self {
        match value {
            ActorError::InvalidHierarchy => Self::InvalidHierarchy,
            ActorError::Dropped | ActorError::NativeReleased | ActorError::NotActive => {
                Self::Closed
            }
        }
    }
}

pub(crate) struct ViewActor {
    actor: ActorRef,
    _main_thread: PhantomData<Rc<()>>,
}
impl ViewActor {
    pub(crate) fn new<D: ApplicationDelegate>(
        application: &Application<D>,
        native: Strong,
    ) -> Self {
        let actor = application.tree().insert_root(native);
        application
            .tree()
            .add_teardown(&actor, |view| unsafe {
                native::send_void(view, native::sel(b"removeFromSuperview\0"))
            })
            .expect("a newly inserted view actor must be live");
        Self {
            actor,
            _main_thread: PhantomData,
        }
    }
    #[cfg(feature = "wgpu")]
    pub(crate) fn from_actor(actor: ActorRef) -> Self {
        actor
            .tree_handle()
            .expect("a newly inserted view actor must have a tree")
            .add_teardown(&actor, |view| unsafe {
                native::send_void(view, native::sel(b"removeFromSuperview\0"))
            })
            .expect("a newly inserted view actor must be live");
        Self {
            actor,
            _main_thread: PhantomData,
        }
    }
    pub(crate) fn as_view(&self) -> ViewRef<'_> {
        ViewRef { actor: &self.actor }
    }
    pub(crate) fn actor(&self) -> &ActorRef {
        &self.actor
    }
}
impl Drop for ViewActor {
    fn drop(&mut self) {
        self.actor.remove();
    }
}

pub struct View {
    inner: ViewActor,
}
impl View {
    pub fn new<D: ApplicationDelegate>(
        application: &Application<D>,
        frame: Rect,
    ) -> Result<Self, ViewError> {
        // SAFETY: NSView's designated frame initializer returns a retained object.
        let native = unsafe {
            let object = native::send_id(native::class(b"NSView\0"), native::sel(b"alloc\0"));
            Strong::from_retained(native::send_id_rect(
                object,
                native::sel(b"initWithFrame:\0"),
                native_rect(frame),
            ))
        }
        .ok_or(ViewError::NativeCreationFailed)?;
        Ok(Self {
            inner: ViewActor::new(application, native),
        })
    }
}
impl AsView for View {
    fn as_view(&self) -> ViewRef<'_> {
        self.inner.as_view()
    }
}

#[derive(Clone, Copy)]
pub struct ViewRef<'view> {
    actor: &'view ActorRef,
}
impl<'view> ViewRef<'view> {
    pub(crate) fn from_actor(actor: &'view ActorRef) -> Self {
        Self { actor }
    }
    pub(crate) fn with<R>(self, operation: impl FnOnce(native::Id) -> R) -> Result<R, ViewError> {
        self.actor.with(operation).map_err(Into::into)
    }
    pub(crate) fn actor(self) -> &'view ActorRef {
        self.actor
    }
    pub fn add_subview(self, child: &impl AsView) -> Result<(), ViewError> {
        let child = child.as_view();
        if !self.actor.same_tree(child.actor) {
            return Err(ViewError::InvalidHierarchy);
        }
        let tree = self.actor.tree_handle().ok_or(ViewError::Closed)?;
        tree.validate_reparent(child.actor, self.actor)
            .map_err(ViewError::from)?;
        self.with(|parent| {
            child.with(|native_child| {
                // SAFETY: Both actors are retained for this synchronous hierarchy update.
                unsafe {
                    native::send_void_id(parent, native::sel(b"addSubview:\0"), native_child)
                };
            })
        })??;
        tree.reparent(child.actor, self.actor)
            .map_err(ViewError::from)
    }
    pub fn remove_from_superview(self) -> Result<(), ViewError> {
        self.with(|view| unsafe {
            native::send_void(view, native::sel(b"removeFromSuperview\0"))
        })?;
        self.actor.move_to_root().map_err(Into::into)
    }
    pub fn set_frame(self, frame: Rect) -> Result<(), ViewError> {
        self.with(|view| unsafe {
            native::send_void_rect(view, native::sel(b"setFrame:\0"), native_rect(frame))
        })
    }
    pub fn set_identifier(self, identifier: Option<&str>) -> Result<(), ViewError> {
        let identifier = identifier.map(native::nsstring);
        self.with(|view| unsafe {
            native::send_void_id(
                view,
                native::sel(b"setAccessibilityIdentifier:\0"),
                identifier.as_ref().map_or(native::NIL, Strong::as_ptr),
            );
        })
    }
    pub fn set_translates_autoresizing_mask_into_constraints(
        self,
        enabled: bool,
    ) -> Result<(), ViewError> {
        self.with(|view| unsafe {
            native::send_void_bool(
                view,
                native::sel(b"setTranslatesAutoresizingMaskIntoConstraints:\0"),
                enabled,
            )
        })
    }
    pub fn leading_anchor(self) -> XAxisAnchor<'view> {
        XAxisAnchor::new(self, LayoutAttribute::Leading)
    }
    pub fn trailing_anchor(self) -> XAxisAnchor<'view> {
        XAxisAnchor::new(self, LayoutAttribute::Trailing)
    }
    pub fn left_anchor(self) -> XAxisAnchor<'view> {
        XAxisAnchor::new(self, LayoutAttribute::Left)
    }
    pub fn right_anchor(self) -> XAxisAnchor<'view> {
        XAxisAnchor::new(self, LayoutAttribute::Right)
    }
    pub fn center_x_anchor(self) -> XAxisAnchor<'view> {
        XAxisAnchor::new(self, LayoutAttribute::CenterX)
    }
    pub fn top_anchor(self) -> YAxisAnchor<'view> {
        YAxisAnchor::new(self, LayoutAttribute::Top)
    }
    pub fn bottom_anchor(self) -> YAxisAnchor<'view> {
        YAxisAnchor::new(self, LayoutAttribute::Bottom)
    }
    pub fn center_y_anchor(self) -> YAxisAnchor<'view> {
        YAxisAnchor::new(self, LayoutAttribute::CenterY)
    }
    pub fn first_baseline_anchor(self) -> YAxisAnchor<'view> {
        YAxisAnchor::new(self, LayoutAttribute::FirstBaseline)
    }
    pub fn last_baseline_anchor(self) -> YAxisAnchor<'view> {
        YAxisAnchor::new(self, LayoutAttribute::LastBaseline)
    }
    pub fn width_anchor(self) -> Dimension<'view> {
        Dimension::new(self, LayoutAttribute::Width)
    }
    pub fn height_anchor(self) -> Dimension<'view> {
        Dimension::new(self, LayoutAttribute::Height)
    }
}

pub trait AsView {
    fn as_view(&self) -> ViewRef<'_>;
    fn add_subview(&self, child: &impl AsView) -> Result<(), ViewError> {
        self.as_view().add_subview(child)
    }
    fn remove_from_superview(&self) -> Result<(), ViewError> {
        self.as_view().remove_from_superview()
    }
    fn set_frame(&self, frame: Rect) -> Result<(), ViewError> {
        self.as_view().set_frame(frame)
    }
    fn set_identifier(&self, identifier: Option<&str>) -> Result<(), ViewError> {
        self.as_view().set_identifier(identifier)
    }
    fn set_translates_autoresizing_mask_into_constraints(
        &self,
        enabled: bool,
    ) -> Result<(), ViewError> {
        self.as_view()
            .set_translates_autoresizing_mask_into_constraints(enabled)
    }
    fn leading_anchor(&self) -> XAxisAnchor<'_> {
        self.as_view().leading_anchor()
    }
    fn trailing_anchor(&self) -> XAxisAnchor<'_> {
        self.as_view().trailing_anchor()
    }
    fn left_anchor(&self) -> XAxisAnchor<'_> {
        self.as_view().left_anchor()
    }
    fn right_anchor(&self) -> XAxisAnchor<'_> {
        self.as_view().right_anchor()
    }
    fn center_x_anchor(&self) -> XAxisAnchor<'_> {
        self.as_view().center_x_anchor()
    }
    fn top_anchor(&self) -> YAxisAnchor<'_> {
        self.as_view().top_anchor()
    }
    fn bottom_anchor(&self) -> YAxisAnchor<'_> {
        self.as_view().bottom_anchor()
    }
    fn center_y_anchor(&self) -> YAxisAnchor<'_> {
        self.as_view().center_y_anchor()
    }
    fn first_baseline_anchor(&self) -> YAxisAnchor<'_> {
        self.as_view().first_baseline_anchor()
    }
    fn last_baseline_anchor(&self) -> YAxisAnchor<'_> {
        self.as_view().last_baseline_anchor()
    }
    fn width_anchor(&self) -> Dimension<'_> {
        self.as_view().width_anchor()
    }
    fn height_anchor(&self) -> Dimension<'_> {
        self.as_view().height_anchor()
    }
}

impl AsView for ViewRef<'_> {
    fn as_view(&self) -> ViewRef<'_> {
        *self
    }
}

pub(crate) fn native_rect(rect: Rect) -> native::Rect {
    native::Rect {
        origin: native::Point {
            x: rect.x,
            y: rect.y,
        },
        size: native::Size {
            width: rect.width,
            height: rect.height,
        },
    }
}
