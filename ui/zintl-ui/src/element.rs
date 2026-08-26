use crate::event::{Event, EventHandlers, EventKind};
use crate::renderer::RenderNode;
use crate::view::Context;
use std::any::TypeId;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ElementKey {
    Integer(u64),
    String(Arc<str>),
}

impl From<u64> for ElementKey {
    fn from(value: u64) -> Self {
        Self::Integer(value)
    }
}

impl From<usize> for ElementKey {
    fn from(value: usize) -> Self {
        Self::Integer(value as u64)
    }
}

impl From<String> for ElementKey {
    fn from(value: String) -> Self {
        Self::String(value.into())
    }
}

impl From<&str> for ElementKey {
    fn from(value: &str) -> Self {
        Self::String(value.into())
    }
}

pub trait BoundBuilder<R: RenderNode>: 'static {
    fn build_children(&mut self, cx: &mut Context<'_>) -> Vec<Element<R>>;
    fn builder_type_id(&self) -> TypeId;
}

pub struct Bound<R: RenderNode> {
    pub(crate) key: Option<ElementKey>,
    pub(crate) builder: Box<dyn BoundBuilder<R>>,
}

pub enum Element<R: RenderNode> {
    Node {
        value: R,
        key: Option<ElementKey>,
        children: Vec<Element<R>>,
        #[doc(hidden)]
        events: EventHandlers,
    },
    Fragment(Vec<Element<R>>),
    Bound(Bound<R>),
}

impl<R: RenderNode> Element<R> {
    pub fn node(value: R) -> Self {
        Self::Node {
            value,
            key: None,
            children: Vec::new(),
            events: EventHandlers::new(),
        }
    }

    pub fn fragment(children: impl IntoIterator<Item = Element<R>>) -> Self {
        Self::Fragment(children.into_iter().collect())
    }

    pub fn with_children(mut self, children: impl IntoIterator<Item = Element<R>>) -> Self {
        match &mut self {
            Self::Node {
                children: current, ..
            } => current.extend(children),
            _ => panic!("only a node can own children"),
        }
        self
    }

    pub fn with_key(mut self, key: impl Into<ElementKey>) -> Self {
        let key = Some(key.into());
        match &mut self {
            Self::Node {
                key: current_key, ..
            } => *current_key = key,
            Self::Bound(bound) => bound.key = key,
            Self::Fragment(_) => panic!("a fragment cannot have a key"),
        }
        self
    }

    /// Registers a semantic event handler on this element.
    ///
    /// The handler belongs to the mounted element and is replaced during
    /// reconciliation. Registering the same kind twice keeps the last handler.
    pub fn on_event(
        mut self,
        kind: EventKind,
        handler: impl for<'a> FnMut(&mut Context<'a>, Event) + 'static,
    ) -> Self {
        match &mut self {
            Self::Node { events, .. } => events.insert(kind, Box::new(handler)),
            _ => panic!("only a node can handle events"),
        }
        self
    }
}

pub trait IntoElement {
    type Output: RenderNode;

    fn into_element(self) -> Element<Self::Output>;
}

impl<R: RenderNode> IntoElement for Element<R> {
    type Output = R;

    fn into_element(self) -> Element<R> {
        self
    }
}

pub trait KeyedElement: IntoElement + Sized {
    fn key(self, key: impl Into<ElementKey>) -> Keyed<Self> {
        Keyed {
            inner: self,
            key: key.into(),
        }
    }
}

impl<T: IntoElement> KeyedElement for T {}

pub struct Keyed<T> {
    inner: T,
    key: ElementKey,
}

impl<T: IntoElement> IntoElement for Keyed<T> {
    type Output = T::Output;

    fn into_element(self) -> Element<Self::Output> {
        self.inner.into_element().with_key(self.key)
    }
}
