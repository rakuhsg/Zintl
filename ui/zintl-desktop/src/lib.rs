use zintl_ui::composer::Composer;
pub use zintl_ui::element::{Element, IntoElement};
use zintl_ui::renderer::{RenderBackend, RenderNode as RenderNodeTrait};
pub use zintl_ui::store::Store;
pub use zintl_ui::view::{Context, View};
pub use zintl_ui_layout::{Axis, LayoutStyle, Size};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    pub const fn new(x: f64, y: f64, width: f64, height: f64) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum RenderNode {
    Text {
        content: String,
        layout: LayoutStyle,
    },
    Button {
        title: String,
        layout: LayoutStyle,
    },
    TextField {
        value: String,
        placeholder: Option<String>,
        binding: Option<Store<String>>,
        layout: LayoutStyle,
    },
    Container {
        layout: LayoutStyle,
    },
    Window {
        bounds: Rect,
        title: String,
    },
}

impl RenderNodeTrait for RenderNode {
    fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Text { .. }, Self::Text { .. })
                | (Self::Button { .. }, Self::Button { .. })
                | (Self::TextField { .. }, Self::TextField { .. })
                | (Self::Container { .. }, Self::Container { .. })
                | (Self::Window { .. }, Self::Window { .. })
        )
    }
}

#[cfg(target_os = "macos")]
impl zintl_ui_appkit::AppKitRenderNode for RenderNode {
    fn appkit_node(&self) -> zintl_ui_appkit::NodeKind {
        use zintl_ui_appkit::{NodeKind, ViewKind};

        match self {
            Self::Text { content, layout } => NodeKind::View {
                kind: ViewKind::Label(content.clone()),
                layout: *layout,
            },
            Self::Button { title, layout } => NodeKind::View {
                kind: ViewKind::Button(title.clone()),
                layout: *layout,
            },
            Self::TextField {
                value,
                placeholder,
                binding,
                layout,
            } => NodeKind::View {
                kind: ViewKind::TextField {
                    value: value.clone(),
                    placeholder: placeholder.clone(),
                    on_change: binding.is_some(),
                },
                layout: *layout,
            },
            Self::Container { layout } => NodeKind::View {
                kind: ViewKind::Container,
                layout: *layout,
            },
            Self::Window { bounds, title } => NodeKind::Window {
                bounds: zintl_ui_appkit::Rect::new(bounds.x, bounds.y, bounds.width, bounds.height),
                title: title.clone(),
            },
        }
    }
}

pub trait Children: 'static {
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

#[derive(Clone)]
pub struct Window<C = Empty> {
    bounds: Rect,
    title: String,
    children: C,
}

impl Window<Empty> {
    pub fn new(bounds: Rect, title: impl Into<String>) -> Self {
        Self {
            bounds,
            title: title.into(),
            children: Empty,
        }
    }
}

impl<C> Window<C> {
    pub fn content<V>(self, content: V) -> Window<(V,)> {
        Window {
            bounds: self.bounds,
            title: self.title,
            children: (content,),
        }
    }
}

impl<C: Children> View for Window<C> {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Window {
            bounds: self.bounds,
            title: self.title.clone(),
        })
        .with_children(self.children.elements())
    }
}

#[derive(Clone)]
pub struct Text {
    content: String,
    layout: LayoutStyle,
}

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        let content = content.into();
        let minimum_width = content.chars().count() as f32 * 7.0;
        Self {
            content,
            layout: LayoutStyle::leaf(Size::new(minimum_width, 20.0)),
        }
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }
}

impl View for Text {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Text {
            content: self.content.clone(),
            layout: self.layout,
        })
    }
}

#[derive(Clone)]
pub struct Button {
    title: String,
    layout: LayoutStyle,
}

impl Button {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            layout: LayoutStyle::leaf(Size::new(80.0, 32.0)),
        }
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }
}

impl View for Button {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Button {
            title: self.title.clone(),
            layout: self.layout,
        })
    }
}

#[derive(Clone)]
pub struct TextField {
    value: TextFieldValue,
    placeholder: Option<String>,
    layout: LayoutStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextFieldValue {
    Literal(String),
    Store(Store<String>),
}

impl From<String> for TextFieldValue {
    fn from(value: String) -> Self {
        Self::Literal(value)
    }
}

impl From<&str> for TextFieldValue {
    fn from(value: &str) -> Self {
        Self::Literal(value.into())
    }
}

impl From<Store<String>> for TextFieldValue {
    fn from(store: Store<String>) -> Self {
        Self::Store(store)
    }
}

impl TextField {
    pub fn new(value: impl Into<TextFieldValue>) -> Self {
        Self {
            value: value.into(),
            placeholder: None,
            layout: LayoutStyle::leaf(Size::new(160.0, 28.0)),
        }
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.placeholder = Some(placeholder.into());
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }
}

impl View for TextField {
    type Output = RenderNode;

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let (value, binding) = match &self.value {
            TextFieldValue::Literal(value) => (value.clone(), None),
            TextFieldValue::Store(store) => (cx.get(*store).clone(), Some(*store)),
        };
        Element::node(RenderNode::TextField {
            value,
            placeholder: self.placeholder.clone(),
            binding,
            layout: self.layout,
        })
    }
}

#[derive(Clone)]
pub struct HStack<C> {
    children: C,
    layout: LayoutStyle,
}

impl<C> HStack<C> {
    pub fn new(children: C) -> Self {
        Self {
            children,
            layout: LayoutStyle::stack(Axis::Horizontal, 8.0),
        }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.layout.gap = spacing;
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }
}

impl<C: Children> View for HStack<C> {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Container {
            layout: self.layout,
        })
        .with_children(self.children.elements())
    }
}

#[derive(Clone)]
pub struct VStack<C> {
    children: C,
    layout: LayoutStyle,
}

impl<C> VStack<C> {
    pub fn new(children: C) -> Self {
        Self {
            children,
            layout: LayoutStyle::stack(Axis::Vertical, 8.0),
        }
    }

    pub fn spacing(mut self, spacing: f32) -> Self {
        self.layout.gap = spacing;
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.layout.minimum_size = size;
        self
    }
}

impl<C: Children> View for VStack<C> {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Container {
            layout: self.layout,
        })
        .with_children(self.children.elements())
    }
}

#[cfg(not(target_os = "macos"))]
struct Node {
    value: Option<RenderNode>,
    parent: Option<usize>,
    children: Vec<usize>,
}

#[cfg(not(target_os = "macos"))]
struct TreeBackend {
    nodes: Vec<Option<Node>>,
}

#[cfg(not(target_os = "macos"))]
impl TreeBackend {
    fn new() -> Self {
        Self {
            nodes: vec![Some(Node {
                value: None,
                parent: None,
                children: Vec::new(),
            })],
        }
    }

    fn node(&self, id: usize) -> &Node {
        self.nodes[id].as_ref().unwrap()
    }

    fn node_mut(&mut self, id: usize) -> &mut Node {
        self.nodes[id].as_mut().unwrap()
    }

    fn children(&self, id: usize) -> &[usize] {
        &self.node(id).children
    }

    fn detach(&mut self, child: usize) {
        let Some(parent) = self.node(child).parent else {
            return;
        };
        self.node_mut(parent)
            .children
            .retain(|candidate| *candidate != child);
        self.node_mut(child).parent = None;
    }
}

#[cfg(not(target_os = "macos"))]
impl RenderBackend<RenderNode> for TreeBackend {
    type NodeId = usize;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn create(&mut self, value: &RenderNode) -> Self::NodeId {
        let id = self.nodes.len();
        self.nodes.push(Some(Node {
            value: Some(value.clone()),
            parent: None,
            children: Vec::new(),
        }));
        id
    }

    fn update(&mut self, node: Self::NodeId, value: &RenderNode) {
        self.node_mut(node).value = Some(value.clone());
    }

    fn insert_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
        self.detach(child);
        let index = index.min(self.node(parent).children.len());
        self.node_mut(parent).children.insert(index, child);
        self.node_mut(child).parent = Some(parent);
    }

    fn remove(&mut self, node: Self::NodeId) {
        self.detach(node);
        self.nodes[node] = None;
    }

    fn move_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
        if self.node(child).parent == Some(parent)
            && self.node(parent).children.get(index) == Some(&child)
        {
            return;
        }
        self.insert_child(parent, index, child);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RenderedNode {
    pub value: RenderNode,
    pub children: Vec<RenderedNode>,
}

#[cfg(target_os = "macos")]
type DesktopBackend = zintl_ui_appkit::AppKitBackend<RenderNode>;

#[cfg(not(target_os = "macos"))]
type DesktopBackend = TreeBackend;

pub struct App {
    composer: Composer<RenderNode, DesktopBackend>,
}

impl App {
    pub fn new<E>(root: E) -> Self
    where
        E: IntoElement<Output = RenderNode>,
    {
        #[cfg(target_os = "macos")]
        let backend = DesktopBackend::new();
        #[cfg(not(target_os = "macos"))]
        let backend = TreeBackend::new();
        let mut composer = Composer::new(backend);
        composer.mount(root);
        Self { composer }
    }

    pub fn render(&self) -> RenderNode {
        self.render_tree().value
    }

    pub fn render_tree(&self) -> RenderedNode {
        let backend = self.composer.backend();
        let root = backend.root();
        let child = *backend
            .children(root)
            .first()
            .expect("the rendered tree must contain a root element");
        rendered_node(backend, child)
    }

    #[cfg(target_os = "macos")]
    pub fn run(self) -> Result<(), AppError> {
        zintl_ui_appkit::run_composer(self.composer, |composer, event| match event {
            zintl_ui_appkit::Event::TextChanged { node, value } => {
                let store = match composer.backend().value(node) {
                    Some(RenderNode::TextField {
                        binding: Some(store),
                        ..
                    }) => *store,
                    _ => return,
                };
                composer.context(|cx| {
                    cx.update(store, |current| *current = value);
                });
                composer.flush();
            }
        })
    }

    #[cfg(test)]
    fn update_text_store(&mut self, store: Store<String>, value: String) {
        self.composer.context(|cx| {
            cx.update(store, |current| *current = value);
        });
        self.composer.flush();
    }
}

#[cfg(target_os = "macos")]
fn rendered_node(backend: &DesktopBackend, id: zintl_ui_appkit::NodeId) -> RenderedNode {
    RenderedNode {
        value: backend
            .value(id)
            .cloned()
            .expect("render nodes always have values"),
        children: backend
            .children(id)
            .iter()
            .map(|child| rendered_node(backend, *child))
            .collect(),
    }
}

#[cfg(not(target_os = "macos"))]
fn rendered_node(backend: &TreeBackend, id: usize) -> RenderedNode {
    let node = backend.node(id);
    RenderedNode {
        value: node.value.clone().expect("render nodes always have values"),
        children: node
            .children
            .iter()
            .map(|child| rendered_node(backend, *child))
            .collect(),
    }
}

#[cfg(target_os = "macos")]
pub use zintl_ui_appkit::AppError;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    struct StoreTextFieldView {
        value: Option<Store<String>>,
    }

    struct BoundLabelView {
        value: Option<Store<String>>,
        renders: Rc<Cell<usize>>,
    }

    impl View for BoundLabelView {
        type Output = RenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.value = Some(cx.store("initial".to_owned()));
        }

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
            self.renders.set(self.renders.get() + 1);
            let store = self
                .value
                .expect("BoundLabelView must be initialized before rendering");
            VStack::new((
                TextField::new(store),
                cx.bind(store, |value| Text::new(format!("Stored value: {value}"))),
            ))
        }
    }

    impl View for StoreTextFieldView {
        type Output = RenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.value = Some(cx.store("initial".to_owned()));
        }

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
            TextField::new(
                self.value
                    .expect("StoreTextFieldView must be initialized before rendering"),
            )
        }
    }

    fn assert_view<T: View<Output = RenderNode>>() {}

    #[test]
    fn desktop_controls_and_stacks_are_views() {
        // Verifies every desktop primitive participates in the View abstraction.
        assert_view::<Text>();
        assert_view::<Button>();
        assert_view::<TextField>();
        assert_view::<HStack<(Text, Button)>>();
        assert_view::<VStack<(TextField,)>>();
    }

    #[test]
    fn window_element_preserves_bounds_and_title() {
        // Verifies the Window view preserves its declarative bounds and title.
        let bounds = Rect::new(10.0, 20.0, 640.0, 480.0);
        let app = App::new(Window::new(bounds, "Zintl"));
        assert_eq!(
            app.render(),
            RenderNode::Window {
                bounds,
                title: "Zintl".into()
            }
        );
    }

    #[test]
    fn stack_render_tree_carries_direction_gap_and_control_sizes() {
        // Verifies desktop layout decisions are retained for the platform backend.
        let app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Zintl")
                .content(HStack::new((Button::new("Save"), TextField::new(""))).spacing(12.0)),
        );
        let tree = app.render_tree();
        let stack = &tree.children[0];

        assert_eq!(
            stack.value,
            RenderNode::Container {
                layout: LayoutStyle::stack(Axis::Horizontal, 12.0),
            }
        );
        assert!(matches!(
            stack.children[0].value,
            RenderNode::Button {
                layout: LayoutStyle {
                    minimum_size: Size {
                        width: 80.0,
                        height: 32.0
                    },
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            stack.children[1].value,
            RenderNode::TextField {
                layout: LayoutStyle {
                    minimum_size: Size {
                        width: 160.0,
                        height: 28.0
                    },
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn text_field_writes_native_input_to_its_store() {
        // Verifies a bound TextField reads from its Store and rerenders after input updates it.
        let mut app = App::new(StoreTextFieldView { value: None });
        let store = match app.render() {
            RenderNode::TextField { value, binding, .. } => {
                assert_eq!(value, "initial");
                binding.expect("a Store-backed TextField must expose its binding")
            }
            node => panic!("expected a TextField, got {node:?}"),
        };

        app.update_text_store(store, "typed value".into());

        let stored = app.composer.context(|cx| cx.get(store).clone());
        assert_eq!(stored, "typed value");
        assert!(matches!(
            app.render(),
            RenderNode::TextField { value, .. } if value == "typed value"
        ));
    }

    #[test]
    fn store_binding_rebuilds_only_its_dependent_element() {
        // Verifies cx.bind updates its label without subscribing the enclosing view.
        let renders = Rc::new(Cell::new(0));
        let mut app = App::new(BoundLabelView {
            value: None,
            renders: renders.clone(),
        });
        let tree = app.render_tree();
        let store = match &tree.children[0].value {
            RenderNode::TextField {
                binding: Some(store),
                ..
            } => *store,
            node => panic!("expected a Store-backed TextField, got {node:?}"),
        };

        app.update_text_store(store, "typed value".into());

        assert_eq!(renders.get(), 1);
        let tree = app.render_tree();
        let RenderNode::Text { content, .. } = &tree.children[1].value else {
            panic!("expected a bound Text, got {:?}", tree.children[1].value);
        };
        assert_eq!(content, "Stored value: typed value");
    }
}
