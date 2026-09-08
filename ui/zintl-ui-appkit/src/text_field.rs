use zintl_ui::element::IntoElement;
use zintl_ui::store::Store;
use zintl_ui::view::{Context, View};
use zintl_ui_layout::{LayoutStyle, Size};

use crate::{Element, Event, EventKind, RenderNode};

#[derive(Clone)]
pub struct TextField {
    value: String,
    binding: Option<Store<String>>,
    placeholder: Option<String>,
    editable: bool,
    selectable: bool,
    bordered: bool,
    draws_background: bool,
    layout: LayoutStyle,
    id: Option<String>,
}

impl TextField {
    pub fn new() -> Self {
        Self::with_string("")
    }

    pub fn with_string(value: impl Into<String>) -> Self {
        Self {
            value: value.into(),
            binding: None,
            placeholder: None,
            editable: true,
            selectable: true,
            bordered: true,
            draws_background: true,
            layout: LayoutStyle::leaf(Size::new(160.0, 28.0)),
            id: None,
        }
    }

    pub fn label_with_string(value: impl Into<String>) -> Self {
        let value = value.into();
        let minimum_width = value.chars().count() as f32 * 7.0;
        Self {
            value,
            binding: None,
            placeholder: None,
            editable: false,
            selectable: false,
            bordered: false,
            draws_background: false,
            layout: LayoutStyle::leaf(Size::new(minimum_width, 20.0)),
            id: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn bind(mut self, store: Store<String>) -> Self {
        self.binding = Some(store);
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }
}

impl Default for TextField {
    fn default() -> Self {
        Self::new()
    }
}

impl View for TextField {
    type Output = RenderNode;

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let value = self
            .binding
            .map(|store| cx.get(store).clone())
            .unwrap_or_else(|| self.value.clone());
        let element = Element::node(RenderNode::NSTextField {
            value,
            placeholder: self.placeholder.clone(),
            editable: self.editable,
            selectable: self.selectable,
            bordered: self.bordered,
            draws_background: self.draws_background,
            layout: self.layout,
            id: self.id.clone(),
        });
        if let Some(store) = self.binding {
            element.on_event(EventKind::TextChanged, move |cx, event| {
                if let Event::TextChanged { value } = event {
                    cx.update(store, |current| *current = value);
                }
            })
        } else {
            element
        }
    }
}
