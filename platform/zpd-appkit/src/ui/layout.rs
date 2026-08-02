use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::ffi;

use super::ViewRef;

#[derive(Clone, Copy)]
#[repr(i32)]
pub(crate) enum LayoutAttribute {
    Left = 1,
    Right = 2,
    Top = 3,
    Bottom = 4,
    Leading = 5,
    Trailing = 6,
    Width = 7,
    Height = 8,
    CenterX = 9,
    CenterY = 10,
    LastBaseline = 11,
    FirstBaseline = 12,
    NotAnAttribute = 0,
}

#[derive(Clone, Copy)]
#[repr(i32)]
enum LayoutRelation {
    LessThanOrEqual = -1,
    Equal = 0,
    GreaterThanOrEqual = 1,
}

#[derive(Clone, Copy)]
pub struct XAxisAnchor<'view> {
    view: ViewRef<'view>,
    attribute: LayoutAttribute,
}

impl<'view> XAxisAnchor<'view> {
    pub(crate) fn new(view: ViewRef<'view>, attribute: LayoutAttribute) -> Self {
        Self { view, attribute }
    }

    pub fn constraint_equal_to(self, other: Self, constant: f64) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::Equal,
            other.view,
            other.attribute,
            1.0,
            constant,
        )
    }

    pub fn constraint_greater_than_or_equal_to(
        self,
        other: Self,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::GreaterThanOrEqual,
            other.view,
            other.attribute,
            1.0,
            constant,
        )
    }

    pub fn constraint_less_than_or_equal_to(
        self,
        other: Self,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::LessThanOrEqual,
            other.view,
            other.attribute,
            1.0,
            constant,
        )
    }
}

#[derive(Clone, Copy)]
pub struct YAxisAnchor<'view> {
    view: ViewRef<'view>,
    attribute: LayoutAttribute,
}

impl<'view> YAxisAnchor<'view> {
    pub(crate) fn new(view: ViewRef<'view>, attribute: LayoutAttribute) -> Self {
        Self { view, attribute }
    }

    pub fn constraint_equal_to(self, other: Self, constant: f64) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::Equal,
            other.view,
            other.attribute,
            1.0,
            constant,
        )
    }

    pub fn constraint_greater_than_or_equal_to(
        self,
        other: Self,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::GreaterThanOrEqual,
            other.view,
            other.attribute,
            1.0,
            constant,
        )
    }

    pub fn constraint_less_than_or_equal_to(
        self,
        other: Self,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::LessThanOrEqual,
            other.view,
            other.attribute,
            1.0,
            constant,
        )
    }
}

#[derive(Clone, Copy)]
pub struct Dimension<'view> {
    view: ViewRef<'view>,
    attribute: LayoutAttribute,
}

impl<'view> Dimension<'view> {
    pub(crate) fn new(view: ViewRef<'view>, attribute: LayoutAttribute) -> Self {
        Self { view, attribute }
    }

    pub fn constraint_equal_to(
        self,
        other: Self,
        multiplier: f64,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::between(
            self.view,
            self.attribute,
            LayoutRelation::Equal,
            other.view,
            other.attribute,
            multiplier,
            constant,
        )
    }

    pub fn constraint_equal_to_constant(self, constant: f64) -> LayoutConstraint<'view> {
        LayoutConstraint::constant(self.view, self.attribute, LayoutRelation::Equal, constant)
    }

    pub fn constraint_greater_than_or_equal_to_constant(
        self,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::constant(
            self.view,
            self.attribute,
            LayoutRelation::GreaterThanOrEqual,
            constant,
        )
    }

    pub fn constraint_less_than_or_equal_to_constant(
        self,
        constant: f64,
    ) -> LayoutConstraint<'view> {
        LayoutConstraint::constant(
            self.view,
            self.attribute,
            LayoutRelation::LessThanOrEqual,
            constant,
        )
    }
}

/// Owns a strong reference to an AppKit `NSLayoutConstraint`.
pub struct LayoutConstraint<'view> {
    raw: NonNull<c_void>,
    _view: PhantomData<&'view ()>,
    _main_thread: PhantomData<Rc<()>>,
}

impl<'view> LayoutConstraint<'view> {
    fn between(
        first: ViewRef<'view>,
        first_attribute: LayoutAttribute,
        relation: LayoutRelation,
        second: ViewRef<'view>,
        second_attribute: LayoutAttribute,
        multiplier: f64,
        constant: f64,
    ) -> Self {
        Self::new(
            first,
            first_attribute,
            relation,
            Some((second, second_attribute)),
            multiplier,
            constant,
        )
    }

    fn constant(
        first: ViewRef<'view>,
        first_attribute: LayoutAttribute,
        relation: LayoutRelation,
        constant: f64,
    ) -> Self {
        Self::new(first, first_attribute, relation, None, 1.0, constant)
    }

    fn new(
        first: ViewRef<'view>,
        first_attribute: LayoutAttribute,
        relation: LayoutRelation,
        second: Option<(ViewRef<'view>, LayoutAttribute)>,
        multiplier: f64,
        constant: f64,
    ) -> Self {
        let (second_view, second_attribute) = second
            .map(|(view, attribute)| (view.as_ptr(), attribute))
            .unwrap_or((std::ptr::null_mut(), LayoutAttribute::NotAnAttribute));
        // SAFETY: The views are live on the AppKit main thread. AppKit returns
        // a +1 retained constraint.
        let raw = unsafe {
            ffi::zintlappkit_layout_constraint_create(
                first.as_ptr(),
                first_attribute as i32,
                relation as i32,
                second_view,
                second_attribute as i32,
                multiplier,
                constant,
            )
        };
        Self {
            raw: NonNull::new(raw).expect("AppKit failed to create a layout constraint"),
            _view: PhantomData,
            _main_thread: PhantomData,
        }
    }

    pub fn activate(constraints: &[Self]) {
        for constraint in constraints {
            constraint.set_active(true);
        }
    }

    pub fn deactivate(constraints: &[Self]) {
        for constraint in constraints {
            constraint.set_active(false);
        }
    }

    pub fn set_active(&self, active: bool) {
        // SAFETY: The retained constraint is live and accessed on main.
        unsafe { ffi::zintlappkit_layout_constraint_set_active(self.raw.as_ptr(), active) };
    }

    pub fn set_priority(&self, priority: f32) {
        // SAFETY: The retained constraint is live and accessed on main.
        unsafe { ffi::zintlappkit_layout_constraint_set_priority(self.raw.as_ptr(), priority) };
    }
}

impl Drop for LayoutConstraint<'_> {
    fn drop(&mut self) {
        // SAFETY: This releases the wrapper's strong reference exactly once.
        // An active constraint remains retained by AppKit's layout engine.
        unsafe { ffi::zintlappkit_release_layout_constraint(self.raw.as_ptr()) };
    }
}
