use std::marker::PhantomData;

use crate::actor::ActorRef;
use zpd_objc::Strong;

use super::{ViewError, ViewRef};

#[derive(Clone, Copy)]
#[repr(i64)]
// Values mirror NSLayoutAttribute.
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
// Values mirror NSLayoutRelation.
pub(crate) enum LayoutRelation {
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
        if !self.view.actor().same_tree(other.view.actor()) {
            return Err(ViewError::InvalidHierarchy);
        }
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
    actor: ActorRef,
    endpoints: Vec<ActorRef>,
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
        let native = first.with(|first_native| {
            second.with(|second_native| unsafe {
                zpd_objc::msg_send!(zpd_objc::class!("NSLayoutConstraint"), zpd_objc::sel!("constraintWithItem:attribute:relatedBy:toItem:attribute:multiplier:constant:"), ((first_native): zpd_objc::Id, (first_attribute as i64): i64, (relation as i64): i64, (second_native): zpd_objc::Id, (second_attribute as i64): i64, (multiplier): f64, (constant): f64) => zpd_objc::Id)
            })
        })??;
        Self::from_native(first, Some(second), native)
    }
    fn constant(
        first: ViewRef<'view>,
        first_attribute: LayoutAttribute,
        relation: LayoutRelation,
        constant: f64,
    ) -> Result<Self, ViewError> {
        let native = first.with(|first_native| unsafe {
            zpd_objc::msg_send!(zpd_objc::class!("NSLayoutConstraint"), zpd_objc::sel!("constraintWithItem:attribute:relatedBy:toItem:attribute:multiplier:constant:"), ((first_native): zpd_objc::Id, (first_attribute as i64): i64, (relation as i64): i64, (zpd_objc::NIL): zpd_objc::Id, (LayoutAttribute::NotAnAttribute as i64): i64, (1.0): f64, (constant): f64) => zpd_objc::Id)
        })?;
        Self::from_native(first, None, native)
    }
    fn from_native(
        first: ViewRef<'view>,
        second: Option<ViewRef<'view>>,
        native_id: zpd_objc::Id,
    ) -> Result<Self, ViewError> {
        // SAFETY: NSLayoutConstraint factory methods return an autoreleased live object.
        let native = unsafe { Strong::retain(native_id) }.ok_or(ViewError::NativeCreationFailed)?;
        let tree = first.actor().tree_handle().ok_or(ViewError::Closed)?;
        let actor = tree
            .insert_child(first.actor(), native)
            .map_err(ViewError::from)?;
        tree.add_dependency(&actor, first.actor())
            .map_err(ViewError::from)?;
        let mut endpoints = vec![first.actor().clone()];
        if let Some(second) = second {
            tree.add_dependency(&actor, second.actor())
                .map_err(ViewError::from)?;
            endpoints.push(second.actor().clone());
        }
        tree.add_teardown(&actor, |constraint| unsafe {
            zpd_objc::msg_send!(constraint, zpd_objc::sel!("setActive:"), ((false): bool) => ())
        })
        .map_err(ViewError::from)?;
        Ok(Self {
            actor,
            endpoints,
            _view: PhantomData,
        })
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
        self.ensure_endpoints()?;
        self.actor
            .with(|constraint| unsafe {
                zpd_objc::msg_send!(constraint, zpd_objc::sel!("setActive:"), ((active): bool) => ())
            })
            .map_err(ViewError::from)
    }
    pub fn set_priority(&self, priority: f32) -> Result<(), ViewError> {
        self.ensure_endpoints()?;
        self.actor
            .with(|constraint| unsafe {
                zpd_objc::msg_send!(constraint, zpd_objc::sel!("setPriority:"), ((priority): f32) => ())
            })
            .map_err(ViewError::from)
    }
    fn ensure_endpoints(&self) -> Result<(), ViewError> {
        if self.endpoints.iter().all(ActorRef::is_alive) {
            Ok(())
        } else {
            Err(ViewError::Closed)
        }
    }
}
impl Drop for LayoutConstraint<'_> {
    fn drop(&mut self) {
        self.actor.remove();
    }
}
