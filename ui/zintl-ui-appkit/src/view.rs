use zintl_ui::element::IntoElement;
use zintl_ui::view::{Context, View as ViewTrait};
use zintl_ui_layout::LayoutStyle;

use crate::{Children, Element, RenderNode};

#[derive(Clone)]
pub struct View<C> {
    children: C,
    layout: LayoutStyle,
    id: Option<String>,
}

impl<C> View<C> {
    pub fn new(layout: LayoutStyle, children: C) -> Self {
        Self {
            children,
            layout,
            id: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }
}

impl<C: Children> ViewTrait for View<C> {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::NSView {
            layout: self.layout,
            id: self.id.clone(),
        })
        .with_children(self.children.elements())
    }
}
