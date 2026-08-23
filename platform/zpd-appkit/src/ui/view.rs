use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::ffi;
use crate::geometry::Rect;
use crate::runloop::{Application, ApplicationDelegate};
use crate::string::NativeOptionalString;

use super::layout::{Dimension, LayoutAttribute, XAxisAnchor, YAxisAnchor};

#[derive(Debug)]
pub enum ViewError {
    NativeCreationFailed,
}

impl std::fmt::Display for ViewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NativeCreationFailed => write!(f, "AppKit failed to create a native view"),
        }
    }
}

impl std::error::Error for ViewError {}

/// Owns a +1 retained pointer to an Objective-C `NSView` instance.
///
/// `raw` points directly to the `NSView` object, or one of its subclasses,
/// rather than to a Rust or Swift bridge wrapper. Dropping this value releases
/// that Objective-C reference without removing the view from its superview.
pub(crate) struct OwnedView {
    raw: NonNull<c_void>,
    _main_thread: PhantomData<Rc<()>>,
}

impl OwnedView {
    pub(crate) unsafe fn from_raw(raw: *mut c_void) -> Result<Self, ViewError> {
        let raw = NonNull::new(raw).ok_or(ViewError::NativeCreationFailed)?;
        Ok(Self {
            raw,
            _main_thread: PhantomData,
        })
    }

    pub(crate) fn as_view(&self) -> ViewRef<'_> {
        // SAFETY: `self` owns a strong native reference for the returned borrow.
        unsafe { ViewRef::from_raw(self.raw.as_ptr()) }
    }
}

impl Drop for OwnedView {
    fn drop(&mut self) {
        // SAFETY: The strong native view reference is released exactly once on
        // the AppKit main thread. Removing it from its superview is explicit.
        unsafe { ffi::zintlappkit_release_view(self.raw.as_ptr()) };
    }
}

/// Owns a strong reference to an AppKit `NSView`.
pub struct View {
    inner: OwnedView,
}

impl View {
    pub fn new<D: ApplicationDelegate>(
        _application: &Application<D>,
        frame: Rect,
    ) -> Result<Self, ViewError> {
        // SAFETY: `Application` is main-thread-bound and the returned handle is
        // a +1 retained AppKit object.
        let inner = unsafe { OwnedView::from_raw(ffi::zintlappkit_create_view(frame))? };
        Ok(Self { inner })
    }
}

impl AsView for View {
    fn as_view(&self) -> ViewRef<'_> {
        self.inner.as_view()
    }
}

/// A borrowed AppKit `NSView` handle.
#[derive(Clone, Copy)]
pub struct ViewRef<'view> {
    raw: NonNull<c_void>,
    _view: PhantomData<&'view ()>,
    _main_thread: PhantomData<Rc<()>>,
}

impl<'view> ViewRef<'view> {
    pub(crate) unsafe fn from_raw(raw: *mut c_void) -> Self {
        Self {
            raw: NonNull::new(raw).expect("AppKit returned a null NSView"),
            _view: PhantomData,
            _main_thread: PhantomData,
        }
    }

    pub(crate) fn as_ptr(self) -> *mut c_void {
        self.raw.as_ptr()
    }

    pub fn add_subview(self, child: &impl AsView) {
        // SAFETY: Both handles refer to live views on the AppKit main thread.
        unsafe {
            ffi::zintlappkit_view_add_subview(self.raw.as_ptr(), child.as_view().as_ptr());
        }
    }

    pub fn remove_from_superview(self) {
        // SAFETY: The borrowed view is live and this call runs on main.
        unsafe { ffi::zintlappkit_view_remove_from_superview(self.raw.as_ptr()) };
    }

    pub fn set_frame(self, frame: Rect) {
        // SAFETY: The borrowed view is live and this call runs on main.
        unsafe { ffi::zintlappkit_view_set_frame(self.raw.as_ptr(), frame) };
    }

    pub fn set_identifier(self, identifier: Option<&str>) {
        // SAFETY: The borrowed view is live, the optional string borrow covers
        // the synchronous call, and this call runs on the main thread.
        unsafe {
            ffi::zintlappkit_view_set_identifier(
                self.raw.as_ptr(),
                NativeOptionalString::from_option(identifier),
            );
        }
    }

    pub fn set_translates_autoresizing_mask_into_constraints(self, enabled: bool) {
        // SAFETY: The borrowed view is live and this call runs on main.
        unsafe {
            ffi::zintlappkit_view_set_translates_autoresizing_mask_into_constraints(
                self.raw.as_ptr(),
                enabled,
            );
        }
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

/// Common `NSView` behavior implemented by native controls and surfaces.
pub trait AsView {
    fn as_view(&self) -> ViewRef<'_>;

    fn add_subview(&self, child: &impl AsView) {
        self.as_view().add_subview(child);
    }

    fn remove_from_superview(&self) {
        self.as_view().remove_from_superview();
    }

    fn set_frame(&self, frame: Rect) {
        self.as_view().set_frame(frame);
    }

    fn set_identifier(&self, identifier: Option<&str>) {
        self.as_view().set_identifier(identifier);
    }

    fn set_translates_autoresizing_mask_into_constraints(&self, enabled: bool) {
        self.as_view()
            .set_translates_autoresizing_mask_into_constraints(enabled);
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
