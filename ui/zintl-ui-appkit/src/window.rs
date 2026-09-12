use zintl_ui::element::{IntoElement, ListFactory};
use zintl_ui::list;
use zintl_ui::view::View;

use crate::{Context, Element, Rect, RenderNode, Sidebar};

#[derive(Clone)]
pub struct Window {
    sidebar: Option<Sidebar>,
    bounds: Rect,
    title: String,
    full_size_content_view: bool,
    id: Option<String>,
    children: ListFactory<RenderNode>,
}

impl Window {
    pub fn new(bounds: Rect, title: impl Into<String>) -> Self {
        Self {
            sidebar: None,
            bounds,
            title: title.into(),
            full_size_content_view: false,
            id: None,
            children: Vec::new(),
        }
    }

    pub fn sidebar(mut self, sidebar: Sidebar) -> Self {
        self.sidebar = Some(sidebar);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn full_size_content_view(mut self) -> Self {
        self.full_size_content_view = true;
        self
    }

    pub fn content<V>(self, content: V) -> Self
    where
        V: Clone + IntoElement<Output = RenderNode> + 'static,
    {
        Self {
            sidebar: self.sidebar,
            bounds: self.bounds,
            title: self.title,
            full_size_content_view: self.full_size_content_view,
            id: self.id,
            children: list![content],
        }
    }
}

impl View for Window {
    type Output = RenderNode;

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let element = Element::node(RenderNode::NSWindow {
            sidebar: self.sidebar.as_ref().map(|sidebar| sidebar.state(cx)),
            bounds: self.bounds,
            title: self.title.clone(),
            full_size_content_view: self.full_size_content_view,
            id: self.id.clone(),
        })
        .with_children([self.children.clone().into_element()]);
        if let Some(sidebar) = &self.sidebar {
            sidebar.route(element)
        } else {
            element
        }
    }
}
