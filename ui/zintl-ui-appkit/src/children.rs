use crate::{Element, RenderNode};
use zintl_ui::element::IntoElement;

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

macro_rules! impl_children_tuple {
    ($($type:ident:$value:ident),+) => {
        impl<$($type),+> Children for ($($type,)+)
        where
            $($type: Clone + IntoElement<Output = RenderNode> + 'static,)+
        {
            fn elements(&self) -> Vec<Element<RenderNode>> {
                let ($($value,)+) = self;
                vec![$($value.clone().into_element(),)+]
            }
        }
    };
}

impl_children_tuple!(A:a);
impl_children_tuple!(A:a, B:b);
impl_children_tuple!(A:a, B:b, C:c);
impl_children_tuple!(A:a, B:b, C:c, D:d);
impl_children_tuple!(A:a, B:b, C:c, D:d, E:e);
impl_children_tuple!(A:a, B:b, C:c, D:d, E:e, F:f);
