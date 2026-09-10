use crate::event::EventHandlers;
use crate::renderer::RenderNode;
use std::any::TypeId;
use std::rc::Rc;
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
    fn build_children<'a>(&mut self, cx: &mut R::Context<'a>) -> Vec<Element<R>>;
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
        events: EventHandlers<R>,
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
        kind: <R::Event as crate::event::Event>::Kind,
        handler: impl for<'a> FnMut(&mut R::Context<'a>, R::Event) + 'static,
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

/// Rebuilds a cloneable [`IntoElement`] value after erasing its concrete type.
#[derive(Clone)]
pub struct ElementFactory<R: RenderNode> {
    build: Rc<dyn Fn() -> Element<R>>,
}

impl<R: RenderNode> ElementFactory<R> {
    /// Captures an element value that can be cloned for repeated rendering.
    pub fn new<E>(element: E) -> Self
    where
        E: Clone + IntoElement<Output = R> + 'static,
    {
        Self {
            build: Rc::new(move || element.clone().into_element()),
        }
    }
}

impl<R: RenderNode> IntoElement for ElementFactory<R> {
    type Output = R;

    fn into_element(self) -> Element<Self::Output> {
        (self.build)()
    }
}

/// A dynamic list of type-erased element factories.
pub type ListFactory<R> = Vec<ElementFactory<R>>;

impl<R: RenderNode> IntoElement for ListFactory<R> {
    type Output = R;

    fn into_element(self) -> Element<Self::Output> {
        Element::fragment(self.into_iter().map(IntoElement::into_element))
    }
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

#[derive(Clone)]
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

/// Creates an extensible [`ListFactory`](crate::element::ListFactory).
#[macro_export]
macro_rules! list {
    ($($element:expr),* $(,)?) => {
        ::std::vec![$($crate::element::ElementFactory::new($element)),*]
    };
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use super::*;
    use crate::view::StoreContext;

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum TestNode {
        Text(&'static str),
        Number(u64),
    }

    #[derive(Clone)]
    struct TestEvent;

    impl crate::event::Event for TestEvent {
        type Kind = ();

        fn kind(&self) -> Self::Kind {}
    }

    impl RenderNode for TestNode {
        type Event = TestEvent;
        type Context<'a> = StoreContext<'a>;

        fn same_kind(&self, other: &Self) -> bool {
            std::mem::discriminant(self) == std::mem::discriminant(other)
        }
    }

    #[derive(Clone)]
    struct Text(&'static str);

    impl IntoElement for Text {
        type Output = TestNode;

        fn into_element(self) -> Element<Self::Output> {
            Element::node(TestNode::Text(self.0))
        }
    }

    #[derive(Clone)]
    struct Number(u64);

    impl IntoElement for Number {
        type Output = TestNode;

        fn into_element(self) -> Element<Self::Output> {
            Element::node(TestNode::Number(self.0))
        }
    }

    #[test]
    fn list_erases_types_without_limiting_length() {
        // Verifies heterogeneous lists preserve order beyond the former tuple limit.
        let factories: ListFactory<TestNode> = crate::list![
            Text("one"),
            Number(2),
            Text("three"),
            Number(4),
            Text("five"),
            Number(6),
            Text("seven"),
            Number(8),
        ];
        let Element::Fragment(elements) = factories.into_element() else {
            panic!("a list factory must produce a fragment");
        };
        let values = elements
            .into_iter()
            .map(|element| match element {
                Element::Node { value, .. } => value,
                _ => panic!("a test factory must produce a node"),
            })
            .collect::<Vec<_>>();

        assert_eq!(
            values,
            vec![
                TestNode::Text("one"),
                TestNode::Number(2),
                TestNode::Text("three"),
                TestNode::Number(4),
                TestNode::Text("five"),
                TestNode::Number(6),
                TestNode::Text("seven"),
                TestNode::Number(8),
            ]
        );
    }

    #[test]
    fn element_factory_reuses_one_evaluated_element() {
        // Verifies list expressions run once while factories can materialize repeatedly.
        let evaluations = Rc::new(Cell::new(0));
        let received = evaluations.clone();
        let factories: ListFactory<TestNode> = crate::list![{
            received.set(received.get() + 1);
            Text("reusable")
        }];
        let first = factories[0].clone().into_element();
        let second = factories[0].clone().into_element();

        assert_eq!(evaluations.get(), 1);
        assert!(matches!(
            first,
            Element::Node {
                value: TestNode::Text("reusable"),
                ..
            }
        ));
        assert!(matches!(
            second,
            Element::Node {
                value: TestNode::Text("reusable"),
                ..
            }
        ));
    }

    #[test]
    fn list_supports_empty_and_keyed_elements() {
        // Verifies empty, keyed, and dynamically extended lists use the public API.
        let empty: ListFactory<TestNode> = crate::list![];
        let mut keyed: ListFactory<TestNode> = crate::list![Text("keyed").key("row"),];
        keyed.push(ElementFactory::new(Number(2)));
        keyed.extend(crate::list![Text("extended")]);

        assert!(empty.is_empty());
        assert_eq!(keyed.len(), 3);
        assert!(matches!(
            keyed[0].clone().into_element(),
            Element::Node {
                key: Some(ElementKey::String(key)),
                ..
            } if key.as_ref() == "row"
        ));
    }
}
