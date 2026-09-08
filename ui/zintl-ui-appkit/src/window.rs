use zintl_ui::element::IntoElement;
use zintl_ui::view::{Context, View};

use crate::{Children, Element, Empty, Rect, RenderNode, Sidebar};

#[derive(Clone)]
pub struct Window<C = Empty> {
    sidebar: Option<Sidebar>,
    bounds: Rect,
    title: String,
    id: Option<String>,
    children: C,
}

impl Window<Empty> {
    pub fn new(bounds: Rect, title: impl Into<String>) -> Self {
        Self {
            sidebar: None,
            bounds,
            title: title.into(),
            id: None,
            children: Empty,
        }
    }
}

impl<C> Window<C> {
    pub fn sidebar(mut self, sidebar: Sidebar) -> Self {
        self.sidebar = Some(sidebar);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn content<V>(self, content: V) -> Window<(V,)> {
        Window {
            sidebar: self.sidebar,
            bounds: self.bounds,
            title: self.title,
            id: self.id,
            children: (content,),
        }
    }
}

impl<C: Children> View for Window<C> {
    type Output = RenderNode;

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let element = Element::node(RenderNode::NSWindow {
            sidebar: self.sidebar.as_ref().map(|sidebar| sidebar.state(cx)),
            bounds: self.bounds,
            title: self.title.clone(),
            id: self.id.clone(),
        })
        .with_children(self.children.elements());
        if let Some(sidebar) = &self.sidebar {
            sidebar.route(element)
        } else {
            element
        }
    }
}
