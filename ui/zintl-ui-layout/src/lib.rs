//! A small, platform-neutral wrapper around Taffy's flexbox layout engine.

use std::error::Error;
use std::fmt;

use taffy::geometry::Size as TaffySize;
use taffy::style::{
    AlignItems, Dimension as TaffyDimension, Display, FlexDirection, JustifyContent,
    LengthPercentage, Style as TaffyStyle,
};
use taffy::{AvailableSpace, NodeId as TaffyNodeId, TaffyTree};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}

impl Size {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Axis {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum LayoutDimension {
    #[default]
    Auto,
    Points(f32),
    Percent(f32),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MainAxisDistribution {
    #[default]
    Start,
    SpaceBetween,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ChildSizing {
    #[default]
    Intrinsic,
    Equal,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CrossAxisAlignment {
    Start,
    #[default]
    Stretch,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutStyle {
    pub minimum_size: Size,
    pub axis: Option<Axis>,
    pub gap: f32,
    pub width: LayoutDimension,
    pub main_axis_distribution: MainAxisDistribution,
    pub child_sizing: ChildSizing,
    pub cross_axis_alignment: CrossAxisAlignment,
}

impl LayoutStyle {
    pub const fn leaf(minimum_size: Size) -> Self {
        Self {
            minimum_size,
            axis: None,
            gap: 0.0,
            width: LayoutDimension::Auto,
            main_axis_distribution: MainAxisDistribution::Start,
            child_sizing: ChildSizing::Intrinsic,
            cross_axis_alignment: CrossAxisAlignment::Stretch,
        }
    }

    pub const fn stack(axis: Axis, gap: f32) -> Self {
        Self {
            minimum_size: Size::new(0.0, 0.0),
            axis: Some(axis),
            gap,
            width: LayoutDimension::Auto,
            main_axis_distribution: MainAxisDistribution::Start,
            child_sizing: ChildSizing::Intrinsic,
            cross_axis_alignment: CrossAxisAlignment::Stretch,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeId(TaffyNodeId);

#[derive(Debug)]
pub struct LayoutError(taffy::TaffyError);

impl fmt::Display for LayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for LayoutError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.0)
    }
}

impl From<taffy::TaffyError> for LayoutError {
    fn from(error: taffy::TaffyError) -> Self {
        Self(error)
    }
}

pub struct LayoutTree {
    tree: TaffyTree,
}

impl Default for LayoutTree {
    fn default() -> Self {
        Self::new()
    }
}

impl LayoutTree {
    pub fn new() -> Self {
        Self {
            tree: TaffyTree::new(),
        }
    }

    pub fn create_node(
        &mut self,
        style: LayoutStyle,
        children: &[NodeId],
    ) -> Result<NodeId, LayoutError> {
        let children: Vec<_> = children.iter().map(|child| child.0).collect();
        if style.child_sizing == ChildSizing::Equal {
            for child in &children {
                let mut child_style = self.tree.style(*child)?.clone();
                child_style.flex_grow = 1.0;
                child_style.flex_basis = TaffyDimension::length(0.0);
                self.tree.set_style(*child, child_style)?;
            }
        }
        Ok(NodeId(
            self.tree
                .new_with_children(to_taffy_style(style), &children)?,
        ))
    }

    pub fn compute(&mut self, root: NodeId, available: Size) -> Result<(), LayoutError> {
        self.tree.compute_layout(
            root.0,
            TaffySize {
                width: AvailableSpace::Definite(available.width),
                height: AvailableSpace::Definite(available.height),
            },
        )?;
        Ok(())
    }

    pub fn layout(&self, node: NodeId) -> Result<Rect, LayoutError> {
        let layout = self.tree.layout(node.0)?;
        Ok(Rect {
            x: layout.location.x,
            y: layout.location.y,
            width: layout.size.width,
            height: layout.size.height,
        })
    }
}

fn to_taffy_style(style: LayoutStyle) -> TaffyStyle {
    let gap = LengthPercentage::length(style.gap);
    TaffyStyle {
        display: Display::Flex,
        size: TaffySize {
            width: match style.width {
                LayoutDimension::Auto => TaffyDimension::auto(),
                LayoutDimension::Points(points) => TaffyDimension::length(points),
                LayoutDimension::Percent(percent) => TaffyDimension::percent(percent),
            },
            height: TaffyDimension::auto(),
        },
        min_size: TaffySize {
            width: TaffyDimension::length(style.minimum_size.width),
            height: TaffyDimension::length(style.minimum_size.height),
        },
        flex_direction: match style.axis.unwrap_or(Axis::Horizontal) {
            Axis::Horizontal => FlexDirection::Row,
            Axis::Vertical => FlexDirection::Column,
        },
        align_items: match style.cross_axis_alignment {
            CrossAxisAlignment::Start => Some(AlignItems::START),
            CrossAxisAlignment::Stretch => Some(AlignItems::STRETCH),
        },
        justify_content: match style.main_axis_distribution {
            MainAxisDistribution::Start => None,
            MainAxisDistribution::SpaceBetween => Some(JustifyContent::SPACE_BETWEEN),
        },
        gap: TaffySize {
            width: gap,
            height: gap,
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_stack_applies_minimum_sizes_and_gap() {
        // Verifies the wrapper maps declarative minimum sizes and horizontal gaps to Taffy.
        let mut tree = LayoutTree::new();
        let first = tree
            .create_node(LayoutStyle::leaf(Size::new(80.0, 32.0)), &[])
            .unwrap();
        let second = tree
            .create_node(LayoutStyle::leaf(Size::new(160.0, 28.0)), &[])
            .unwrap();
        let root = tree
            .create_node(LayoutStyle::stack(Axis::Horizontal, 12.0), &[first, second])
            .unwrap();

        tree.compute(root, Size::new(400.0, 100.0)).unwrap();

        assert_eq!(tree.layout(first).unwrap().x, 0.0);
        assert_eq!(tree.layout(first).unwrap().width, 80.0);
        assert_eq!(tree.layout(second).unwrap().x, 92.0);
        assert_eq!(tree.layout(second).unwrap().width, 160.0);
    }

    #[test]
    fn percentage_width_fills_a_definite_parent() {
        // Verifies a percentage width resolves against the parent's declared width.
        let mut tree = LayoutTree::new();
        let mut child_style = LayoutStyle::stack(Axis::Vertical, 0.0);
        child_style.width = LayoutDimension::Percent(1.0);
        let child = tree.create_node(child_style, &[]).unwrap();
        let mut root_style = LayoutStyle::stack(Axis::Vertical, 0.0);
        root_style.width = LayoutDimension::Points(400.0);
        root_style.cross_axis_alignment = CrossAxisAlignment::Start;
        let root = tree.create_node(root_style, &[child]).unwrap();

        tree.compute(root, Size::new(400.0, 100.0)).unwrap();

        assert_eq!(tree.layout(child).unwrap().width, 400.0);
    }

    #[test]
    fn space_between_distributes_space_beyond_the_minimum_gap() {
        // Verifies space-between keeps child widths and places the outer children at both edges.
        let mut tree = LayoutTree::new();
        let first = tree
            .create_node(LayoutStyle::leaf(Size::new(80.0, 20.0)), &[])
            .unwrap();
        let second = tree
            .create_node(LayoutStyle::leaf(Size::new(160.0, 20.0)), &[])
            .unwrap();
        let mut root_style = LayoutStyle::stack(Axis::Horizontal, 12.0);
        root_style.width = LayoutDimension::Points(400.0);
        root_style.main_axis_distribution = MainAxisDistribution::SpaceBetween;
        let root = tree.create_node(root_style, &[first, second]).unwrap();

        tree.compute(root, Size::new(400.0, 100.0)).unwrap();

        assert_eq!(tree.layout(first).unwrap().x, 0.0);
        assert_eq!(tree.layout(second).unwrap().x, 240.0);
        assert_eq!(tree.layout(second).unwrap().width, 160.0);
    }

    #[test]
    fn equal_children_share_available_width() {
        // Verifies equal sizing divides the width after subtracting the declared gap.
        let mut tree = LayoutTree::new();
        let first = tree
            .create_node(LayoutStyle::leaf(Size::new(80.0, 20.0)), &[])
            .unwrap();
        let second = tree
            .create_node(LayoutStyle::leaf(Size::new(160.0, 20.0)), &[])
            .unwrap();
        let mut root_style = LayoutStyle::stack(Axis::Horizontal, 20.0);
        root_style.width = LayoutDimension::Points(400.0);
        root_style.child_sizing = ChildSizing::Equal;
        let root = tree.create_node(root_style, &[first, second]).unwrap();

        tree.compute(root, Size::new(400.0, 100.0)).unwrap();

        assert_eq!(tree.layout(first).unwrap().width, 190.0);
        assert_eq!(tree.layout(second).unwrap().width, 190.0);
    }

    #[test]
    fn equal_children_preserve_minimum_widths_when_constrained() {
        // Verifies minimum widths take precedence when equal shares would be too small.
        let mut tree = LayoutTree::new();
        let first = tree
            .create_node(LayoutStyle::leaf(Size::new(80.0, 20.0)), &[])
            .unwrap();
        let second = tree
            .create_node(LayoutStyle::leaf(Size::new(160.0, 20.0)), &[])
            .unwrap();
        let mut root_style = LayoutStyle::stack(Axis::Horizontal, 20.0);
        root_style.width = LayoutDimension::Points(200.0);
        root_style.child_sizing = ChildSizing::Equal;
        let root = tree.create_node(root_style, &[first, second]).unwrap();

        tree.compute(root, Size::new(200.0, 100.0)).unwrap();

        assert!(tree.layout(first).unwrap().width >= 80.0);
        assert!(tree.layout(second).unwrap().width >= 160.0);
    }
}
