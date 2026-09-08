use std::rc::Rc;

use zintl_ui::element::IntoElement;
use zintl_ui::view::{Context, View};
use zintl_ui_layout::{LayoutStyle, Size};

use crate::{Element, Event, EventKind, RenderNode};

type Action = dyn for<'a> Fn(&mut Context<'a>);

#[derive(Clone)]
pub struct Button {
    title: String,
    layout: LayoutStyle,
    id: Option<String>,
    action: Option<Rc<Action>>,
}

impl Button {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            layout: LayoutStyle::leaf(Size::new(80.0, 32.0)),
            id: None,
            action: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }

    /// Schedules `action` when this button is activated by AppKit.
    pub fn on_click(mut self, action: impl for<'a> Fn(&mut Context<'a>) + 'static) -> Self {
        self.action = Some(Rc::new(action));
        self
    }
}

impl View for Button {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let element = Element::node(RenderNode::NSButton {
            title: self.title.clone(),
            layout: self.layout,
            id: self.id.clone(),
        });
        if let Some(action) = self.action.clone() {
            element.on_event(EventKind::Activated, move |cx, event| {
                if event == Event::Activated {
                    action(cx);
                }
            })
        } else {
            element
        }
    }
}
