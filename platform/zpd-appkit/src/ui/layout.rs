use std::marker::PhantomData;

use crate::native::{self, Strong};

use super::{ViewError, ViewRef};

#[derive(Clone, Copy)]
#[repr(i64)]
pub(crate) enum LayoutAttribute {
    NotAnAttribute = 0,
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
}
#[derive(Clone, Copy)]
#[repr(i64)]
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
    pub fn constraint_equal_to(
        self,
        other: Self,
        constant: f64,
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    pub fn constraint_equal_to(
        self,
        other: Self,
        constant: f64,
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    pub fn constraint_equal_to_constant(
        self,
        constant: f64,
    ) -> Result<LayoutConstraint<'view>, ViewError> {
        LayoutConstraint::constant(self.view, self.attribute, LayoutRelation::Equal, constant)
    }
    pub fn constraint_greater_than_or_equal_to_constant(
        self,
        constant: f64,
    ) -> Result<LayoutConstraint<'view>, ViewError> {
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
    ) -> Result<LayoutConstraint<'view>, ViewError> {
        LayoutConstraint::constant(
            self.view,
            self.attribute,
            LayoutRelation::LessThanOrEqual,
            constant,
        )
    }
}

pub struct LayoutConstraint<'view> {
    native: Strong,
    _view: PhantomData<&'view ()>,
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
    ) -> Result<Self, ViewError> {
        let second_native = second.with(|id| id)?;
        Self::new(
            first,
            first_attribute,
            relation,
            second_native,
            second_attribute,
            multiplier,
            constant,
        )
    }
    fn constant(
        first: ViewRef<'view>,
        first_attribute: LayoutAttribute,
        relation: LayoutRelation,
        constant: f64,
    ) -> Result<Self, ViewError> {
        Self::new(
            first,
            first_attribute,
            relation,
            native::NIL,
            LayoutAttribute::NotAnAttribute,
            1.0,
            constant,
        )
    }
    fn new(
        first: ViewRef<'view>,
        first_attribute: LayoutAttribute,
        relation: LayoutRelation,
        second: native::Id,
        second_attribute: LayoutAttribute,
        multiplier: f64,
        constant: f64,
    ) -> Result<Self, ViewError> {
        first.with(|first| unsafe {
            let value = native::send_constraint(native::class(b"NSLayoutConstraint\0"), native::sel(b"constraintWithItem:attribute:relatedBy:toItem:attribute:multiplier:constant:\0"), first, first_attribute as i64, relation as i64, second, second_attribute as i64, multiplier, constant);
            Strong::retain(value).map(|native| Self { native, _view: PhantomData }).ok_or(ViewError::NativeCreationFailed)
        })?
    }
    pub fn activate(constraints: &[Self]) -> Result<(), ViewError> {
        for value in constraints {
            value.set_active(true)?;
        }
        Ok(())
    }
    pub fn deactivate(constraints: &[Self]) -> Result<(), ViewError> {
        for value in constraints {
            value.set_active(false)?;
        }
        Ok(())
    }
    pub fn set_active(&self, active: bool) -> Result<(), ViewError> {
        unsafe {
            native::send_void_bool(self.native.as_ptr(), native::sel(b"setActive:\0"), active)
        };
        Ok(())
    }
    pub fn set_priority(&self, priority: f32) -> Result<(), ViewError> {
        unsafe {
            native::send_void_f32(
                self.native.as_ptr(),
                native::sel(b"setPriority:\0"),
                priority,
            )
        };
        Ok(())
    }
}
