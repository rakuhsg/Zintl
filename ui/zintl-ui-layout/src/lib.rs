//! A small, platform-neutral wrapper around Taffy's flexbox layout engine.

use std::error::Error;
use std::fmt;

use taffy::geometry::Size as TaffySize;
use taffy::style::{Dimension, Display, FlexDirection, LengthPercentage, Style as TaffyStyle};
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

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LayoutStyle {
    pub minimum_size: Size,
    pub axis: Option<Axis>,
    pub gap: f32,
}

impl LayoutStyle {
    pub const fn leaf(minimum_size: Size) -> Self {
        Self {
            minimum_size,
            axis: None,
            gap: 0.0,
        }
    }

    pub const fn stack(axis: Axis, gap: f32) -> Self {
        Self {
            minimum_size: Size::new(0.0, 0.0),
            axis: Some(axis),
            gap,
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
        min_size: TaffySize {
            width: Dimension::length(style.minimum_size.width),
            height: Dimension::length(style.minimum_size.height),
        },
        flex_direction: match style.axis.unwrap_or(Axis::Horizontal) {
            Axis::Horizontal => FlexDirection::Row,
            Axis::Vertical => FlexDirection::Column,
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
}
