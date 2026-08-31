use std::rc::Rc;
use zintl_ui::composer::Composer;
pub use zintl_ui::element::{Element, IntoElement};
use zintl_ui::event::Event as EventTrait;
#[cfg(not(target_os = "macos"))]
use zintl_ui::event::EventRouteId;
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

/// Selects the semantic desktop event handled by an element route.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum EventKind {
    Activated,
    TextChanged,
    WindowCreated,
    WindowDidResize,
    WindowWillClose,
    WindowDidClose,
}

/// A semantic event produced by a desktop backend.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Activated,
    TextChanged { value: String },
    WindowCreated,
    WindowDidResize,
    WindowWillClose,
    WindowDidClose,
}

impl EventTrait for Event {
    type Kind = EventKind;

    fn kind(&self) -> Self::Kind {
        match self {
            Self::Activated => EventKind::Activated,
            Self::TextChanged { .. } => EventKind::TextChanged,
            Self::WindowCreated => EventKind::WindowCreated,
            Self::WindowDidResize => EventKind::WindowDidResize,
            Self::WindowWillClose => EventKind::WindowWillClose,
            Self::WindowDidClose => EventKind::WindowDidClose,
        }
    }
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
        id: Option<String>,
    },
    Button {
        title: String,
        layout: LayoutStyle,
        id: Option<String>,
    },
    TextField {
        value: String,
        placeholder: Option<String>,
        layout: LayoutStyle,
        id: Option<String>,
    },
    Container {
        layout: LayoutStyle,
        id: Option<String>,
    },
    Window {
        bounds: Rect,
        title: String,
        id: Option<String>,
    },
}

impl RenderNodeTrait for RenderNode {
    type Event = Event;

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
            Self::Text {
                content,
                layout,
                id,
            } => NodeKind::View {
                kind: ViewKind::Label(content.clone()),
                layout: *layout,
                id: id.clone(),
            },
            Self::Button { title, layout, id } => NodeKind::View {
                kind: ViewKind::Button(title.clone()),
                layout: *layout,
                id: id.clone(),
            },
            Self::TextField {
                value,
                placeholder,
                layout,
                id,
            } => NodeKind::View {
                kind: ViewKind::TextField {
                    value: value.clone(),
                    placeholder: placeholder.clone(),
                },
                layout: *layout,
                id: id.clone(),
            },
            Self::Container { layout, id } => NodeKind::View {
                kind: ViewKind::Container,
                layout: *layout,
                id: id.clone(),
            },
            Self::Window { bounds, title, id } => NodeKind::Window {
                bounds: zintl_ui_appkit::Rect::new(bounds.x, bounds.y, bounds.width, bounds.height),
                title: title.clone(),
                id: id.clone(),
            },
        }
    }

    fn appkit_event(event: zintl_ui_appkit::AppKitEvent) -> Self::Event {
        match event {
            zintl_ui_appkit::AppKitEvent::Created => Event::WindowCreated,
            zintl_ui_appkit::AppKitEvent::DidResize => Event::WindowDidResize,
            zintl_ui_appkit::AppKitEvent::WillClose => Event::WindowWillClose,
            zintl_ui_appkit::AppKitEvent::DidClose => Event::WindowDidClose,
            zintl_ui_appkit::AppKitEvent::ButtonClicked => Event::Activated,
            zintl_ui_appkit::AppKitEvent::TextChanged { value } => Event::TextChanged { value },
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
    id: Option<String>,
    children: C,
}

impl Window<Empty> {
    pub fn new(bounds: Rect, title: impl Into<String>) -> Self {
        Self {
            bounds,
            title: title.into(),
            id: None,
            children: Empty,
        }
    }
}

impl<C> Window<C> {
    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
    }

    pub fn content<V>(self, content: V) -> Window<(V,)> {
        Window {
            bounds: self.bounds,
            title: self.title,
            id: self.id,
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
            id: self.id.clone(),
        })
        .with_children(self.children.elements())
    }
}

#[derive(Clone)]
pub struct Text {
    content: String,
    layout: LayoutStyle,
    id: Option<String>,
}

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        let content = content.into();
        let minimum_width = content.chars().count() as f32 * 7.0;
        Self {
            content,
            layout: LayoutStyle::leaf(Size::new(minimum_width, 20.0)),
            id: None,
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
}

impl View for Text {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        Element::node(RenderNode::Text {
            content: self.content.clone(),
            layout: self.layout,
            id: self.id.clone(),
        })
    }
}

#[derive(Clone)]
pub struct Button {
    title: String,
    layout: LayoutStyle,
    id: Option<String>,
    action: Option<Rc<dyn for<'a> Fn(&mut Context<'a>)>>,
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

    /// Schedules `action` when this Button is activated by the platform.
    pub fn on_click(mut self, action: impl for<'a> Fn(&mut Context<'a>) + 'static) -> Self {
        self.action = Some(Rc::new(action));
        self
    }
}

impl View for Button {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let element = Element::node(RenderNode::Button {
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

#[derive(Clone)]
pub struct TextField {
    binding: Option<Store<String>>,
    placeholder: Option<String>,
    layout: LayoutStyle,
    id: Option<String>,
}

impl TextField {
    pub fn new() -> Self {
        Self {
            binding: None,
            placeholder: None,
            layout: LayoutStyle::leaf(Size::new(160.0, 28.0)),
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

impl View for TextField {
    type Output = RenderNode;

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        let value = self
            .binding
            .map(|store| cx.get(store).clone())
            .unwrap_or_default();
        let element = Element::node(RenderNode::TextField {
            value,
            placeholder: self.placeholder.clone(),
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

#[derive(Clone)]
pub struct HStack<C> {
    children: C,
    layout: LayoutStyle,
    id: Option<String>,
}

impl<C> HStack<C> {
    pub fn new(children: C) -> Self {
        Self {
            children,
            layout: LayoutStyle::stack(Axis::Horizontal, 8.0),
            id: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
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
            id: self.id.clone(),
        })
        .with_children(self.children.elements())
    }
}

#[derive(Clone)]
pub struct VStack<C> {
    children: C,
    layout: LayoutStyle,
    id: Option<String>,
}

impl<C> VStack<C> {
    pub fn new(children: C) -> Self {
        Self {
            children,
            layout: LayoutStyle::stack(Axis::Vertical, 8.0),
            id: None,
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = Some(id.into());
        self
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
            id: self.id.clone(),
        })
        .with_children(self.children.elements())
    }
}

#[cfg(not(target_os = "macos"))]
struct Node {
    value: Option<RenderNode>,
    parent: Option<usize>,
    children: Vec<usize>,
    event_route: Option<EventRouteId>,
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
                event_route: None,
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

    fn create(&mut self, value: &RenderNode, event_route: Option<EventRouteId>) -> Self::NodeId {
        let id = self.nodes.len();
        self.nodes.push(Some(Node {
            value: Some(value.clone()),
            parent: None,
            children: Vec::new(),
            event_route,
        }));
        id
    }

    fn update(&mut self, node: Self::NodeId, value: &RenderNode) {
        self.node_mut(node).value = Some(value.clone());
    }

    fn set_event_route(&mut self, node: Self::NodeId, event_route: Option<EventRouteId>) {
        self.node_mut(node).event_route = event_route;
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
        zintl_ui_appkit::run_composer(self.composer)
    }

    #[cfg(test)]
    fn update_text_store(&mut self, store: Store<String>, value: String) {
        self.composer.context(|cx| {
            cx.update(store, |current| *current = value);
        });
        self.composer.flush();
    }

    #[cfg(test)]
    fn dispatch_root_event(&mut self, event: Event) -> bool {
        let route = root_event_route(self.composer.backend())
            .expect("the rendered root must have an event route");
        self.composer.dispatch_event(route, event)
    }
}

#[cfg(all(test, target_os = "macos"))]
fn root_event_route(backend: &DesktopBackend) -> Option<zintl_ui::event::EventRouteId> {
    let root = backend.root();
    let child = *backend.children(root).first()?;
    backend.event_route(child)
}

#[cfg(all(test, not(target_os = "macos")))]
fn root_event_route(backend: &TreeBackend) -> Option<zintl_ui::event::EventRouteId> {
    let child = *backend.children(backend.root()).first()?;
    backend.node(child).event_route
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
        captured: Rc<Cell<Option<Store<String>>>>,
    }

    struct BoundLabelView {
        value: Option<Store<String>>,
        renders: Rc<Cell<usize>>,
        captured: Rc<Cell<Option<Store<String>>>>,
    }

    impl View for BoundLabelView {
        type Output = RenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.value = Some(cx.store("initial".to_owned()));
            self.captured.set(self.value);
        }

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
            self.renders.set(self.renders.get() + 1);
            let store = self
                .value
                .expect("BoundLabelView must be initialized before rendering");
            VStack::new((
                TextField::new().bind(store),
                cx.watch(store, |value| Text::new(format!("Stored value: {value}"))),
            ))
        }
    }

    impl View for StoreTextFieldView {
        type Output = RenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.value = Some(cx.store("initial".to_owned()));
            self.captured.set(self.value);
        }

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
            TextField::new().bind(
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
                title: "Zintl".into(),
                id: None,
            }
        );
    }

    #[test]
    fn stack_render_tree_carries_direction_gap_and_control_sizes() {
        // Verifies desktop layout decisions are retained for the platform backend.
        let app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Zintl")
                .content(HStack::new((Button::new("Save"), TextField::new())).spacing(12.0)),
        );
        let tree = app.render_tree();
        let stack = &tree.children[0];

        assert_eq!(
            stack.value,
            RenderNode::Container {
                layout: LayoutStyle::stack(Axis::Horizontal, 12.0),
                id: None,
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
        let captured = Rc::new(Cell::new(None));
        let mut app = App::new(StoreTextFieldView {
            value: None,
            captured: captured.clone(),
        });
        assert!(matches!(app.render(), RenderNode::TextField { value, .. } if value == "initial"));
        let store = captured.get().expect("init must publish the test Store");

        assert!(app.dispatch_root_event(Event::TextChanged {
            value: "typed value".into(),
        }));

        let stored = app.composer.context(|cx| cx.get(store).clone());
        assert_eq!(stored, "typed value");
        assert!(matches!(
            app.render(),
            RenderNode::TextField { value, .. } if value == "typed value"
        ));
    }

    #[test]
    fn button_dispatches_its_registered_action() {
        // Verifies Button activation reaches its Element-owned action without a backend node lookup.
        let activations = Rc::new(Cell::new(0));
        let received = activations.clone();
        let mut app = App::new(Button::new("Save").on_click(move |_| {
            received.set(received.get() + 1);
        }));

        assert!(app.dispatch_root_event(Event::Activated));
        assert_eq!(activations.get(), 1);
    }

    #[test]
    fn view_ids_are_preserved_in_the_render_tree() {
        // Verifies stable IDs survive declarative rendering for platform accessibility backends.
        let app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Zintl")
                .id("main-window")
                .content(TextField::new().id("name-input")),
        );
        let tree = app.render_tree();

        assert!(matches!(
            &tree.value,
            RenderNode::Window { id: Some(id), .. } if id == "main-window"
        ));
        assert!(matches!(
            &tree.children[0].value,
            RenderNode::TextField { id: Some(id), .. } if id == "name-input"
        ));
    }

    #[test]
    fn store_watcher_rebuilds_only_its_dependent_element() {
        // Verifies cx.watch updates its label without subscribing the enclosing view.
        let renders = Rc::new(Cell::new(0));
        let captured = Rc::new(Cell::new(None));
        let mut app = App::new(BoundLabelView {
            value: None,
            renders: renders.clone(),
            captured: captured.clone(),
        });
        let tree = app.render_tree();
        assert!(matches!(
            &tree.children[0].value,
            RenderNode::TextField { .. }
        ));
        let store = captured.get().expect("init must publish the test Store");

        app.update_text_store(store, "typed value".into());

        assert_eq!(renders.get(), 1);
        let tree = app.render_tree();
        let RenderNode::Text { content, .. } = &tree.children[1].value else {
            panic!("expected a bound Text, got {:?}", tree.children[1].value);
        };
        assert_eq!(content, "Stored value: typed value");
    }
}
