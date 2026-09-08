use crate::{Element, RenderNode};
use zintl_ui::element::{ElementFactory, IntoElement};

pub trait Children: Clone + 'static {
    fn elements(&self) -> Vec<Element<RenderNode>>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Empty;

impl Children for Empty {
    fn elements(&self) -> Vec<Element<RenderNode>> {
        Vec::new()
    }
}

impl Children for Vec<ElementFactory<RenderNode>> {
    fn elements(&self) -> Vec<Element<RenderNode>> {
        self.iter()
            .cloned()
            .map(IntoElement::into_element)
            .collect()
    }
}
