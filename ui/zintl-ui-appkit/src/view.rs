use zintl_ui::element::{IntoElement, ListFactory};
use zintl_ui::view::View as ViewTrait;
use zintl_ui_layout::LayoutStyle;

use crate::{Context, Element, RenderNode};

#[derive(Clone)]
pub struct View {
    children: ListFactory<RenderNode>,
    layout: LayoutStyle,
    id: Option<String>,
}

impl View {
    pub fn new(layout: LayoutStyle, children: ListFactory<RenderNode>) -> Self {
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

impl ViewTrait for View {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::NSView {
            layout: self.layout,
            id: self.id.clone(),
        })
        .with_children([self.children.clone().into_element()])
    }
}
