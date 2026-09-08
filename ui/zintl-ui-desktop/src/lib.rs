use zintl_ui::composer::Composer;
pub use zintl_ui::element::{Element, IntoElement};
#[cfg(any(not(target_os = "macos"), test))]
use zintl_ui::event::EventRouteId;
use zintl_ui::renderer::RenderBackend;
pub use zintl_ui::store::Store;
pub use zintl_ui::view::{Context, View};
pub use zintl_ui_appkit::{
    Children, Empty, Event, EventKind, Rect, RenderNode, Sidebar, SidebarItem, SidebarSection,
    SidebarState,
};
pub use zintl_ui_layout::{
    Axis, ChildSizing, CrossAxisAlignment, LayoutDimension, LayoutStyle, MainAxisDistribution, Size,
};

#[derive(Clone)]
pub struct Window<C = Empty> {
    inner: zintl_ui_appkit::Window<C>,
}

impl Window<Empty> {
    pub fn new(bounds: Rect, title: impl Into<String>) -> Self {
        Self {
            inner: zintl_ui_appkit::Window::new(bounds, title),
        }
    }
}

impl<C> Window<C> {
    pub fn sidebar(mut self, sidebar: Sidebar) -> Self {
        self.inner = self.inner.sidebar(sidebar);
        self
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.inner = self.inner.id(id);
        self
    }

    pub fn content<V>(self, content: V) -> Window<(V,)> {
        Window {
            inner: self.inner.content(content),
        }
    }
}

impl<C: Children> View for Window<C> {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        self.inner.clone().into_element()
    }
}

#[derive(Clone)]
pub struct Text {
    inner: zintl_ui_appkit::TextField,
}

impl Text {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            inner: zintl_ui_appkit::TextField::label_with_string(content),
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.inner = self.inner.id(id);
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.inner = self.inner.minimum_size(size);
        self
    }
}

impl View for Text {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        self.inner.clone().into_element()
    }
}

#[derive(Clone)]
pub struct Button {
    inner: zintl_ui_appkit::Button,
}

impl Button {
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            inner: zintl_ui_appkit::Button::new(title),
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.inner = self.inner.id(id);
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.inner = self.inner.minimum_size(size);
        self
    }

    pub fn on_click(mut self, action: impl for<'a> Fn(&mut Context<'a>) + 'static) -> Self {
        self.inner = self.inner.on_click(action);
        self
    }
}

impl View for Button {
    type Output = RenderNode;

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        self.inner.clone().into_element()
    }
}

#[derive(Clone)]
pub struct TextField {
    inner: zintl_ui_appkit::TextField,
}

impl TextField {
    pub fn new() -> Self {
        Self {
            inner: zintl_ui_appkit::TextField::new(),
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.inner = self.inner.id(id);
        self
    }

    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Self {
        self.inner = self.inner.placeholder(placeholder);
        self
    }

    pub fn bind(mut self, store: Store<String>) -> Self {
        self.inner = self.inner.bind(store);
        self
    }

    pub fn minimum_size(mut self, size: Size) -> Self {
        self.inner = self.inner.minimum_size(size);
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

    fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
        self.inner.clone().into_element()
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

    pub fn fill_width(mut self) -> Self {
        self.layout.width = LayoutDimension::Percent(1.0);
        self
    }

    pub fn space_between(mut self) -> Self {
        self.layout.main_axis_distribution = MainAxisDistribution::SpaceBetween;
        self
    }

    pub fn equal_width_children(mut self) -> Self {
        self.layout.child_sizing = ChildSizing::Equal;
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
        let view = zintl_ui_appkit::View::new(self.layout, self.children.clone());
        match &self.id {
            Some(id) => view.id(id.clone()).into_element(),
            None => view.into_element(),
        }
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

    pub fn fill_width(mut self) -> Self {
        self.layout.width = LayoutDimension::Percent(1.0);
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
        let view = zintl_ui_appkit::View::new(self.layout, self.children.clone());
        match &self.id {
            Some(id) => view.id(id.clone()).into_element(),
            None => view.into_element(),
        }
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
        self.composer
            .context(|cx| cx.update(store, |current| *current = value));
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
fn root_event_route(backend: &DesktopBackend) -> Option<EventRouteId> {
    let root = backend.root();
    let child = *backend.children(root).first()?;
    backend.event_route(child)
}

#[cfg(all(test, not(target_os = "macos")))]
fn root_event_route(backend: &TreeBackend) -> Option<EventRouteId> {
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

    struct BoundTextView {
        value: Option<Store<String>>,
        renders: Rc<Cell<usize>>,
        captured: Rc<Cell<Option<Store<String>>>>,
    }

    impl View for BoundTextView {
        type Output = RenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.value = Some(cx.store("initial".to_owned()));
            self.captured.set(self.value);
        }

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
            self.renders.set(self.renders.get() + 1);
            let store = self
                .value
                .expect("BoundTextView must be initialized before rendering");
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
    fn text_and_text_field_share_the_native_render_kind() {
        // Verifies desktop text wrappers both render through NSTextField configuration.
        let label = App::new(Text::new("Label")).render();
        let field = App::new(TextField::new()).render();

        assert!(matches!(
            label,
            RenderNode::NSTextField {
                editable: false,
                bordered: false,
                ..
            }
        ));
        assert!(matches!(
            field,
            RenderNode::NSTextField {
                editable: true,
                bordered: true,
                ..
            }
        ));
    }

    #[test]
    fn window_element_preserves_bounds_and_title() {
        // Verifies the Window wrapper preserves its declarative bounds and title.
        let bounds = Rect::new(10.0, 20.0, 640.0, 480.0);
        let app = App::new(Window::new(bounds, "Zintl"));
        assert_eq!(
            app.render(),
            RenderNode::NSWindow {
                sidebar: None,
                bounds,
                title: "Zintl".into(),
                id: None,
            }
        );
    }

    #[test]
    fn stack_render_tree_carries_layout_and_control_sizes() {
        // Verifies stack wrappers preserve layout policy while using native render nodes.
        let app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Zintl")
                .content(HStack::new((Button::new("Save"), TextField::new())).spacing(12.0)),
        );
        let tree = app.render_tree();
        let stack = &tree.children[0];

        assert_eq!(
            stack.value,
            RenderNode::NSView {
                layout: LayoutStyle::stack(Axis::Horizontal, 12.0),
                id: None,
            }
        );
        assert!(matches!(
            stack.children[0].value,
            RenderNode::NSButton {
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
            RenderNode::NSTextField {
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
    fn stack_builders_preserve_width_and_child_distribution() {
        // Verifies desktop stack options reach the wrapped AppKit NSView layout.
        let app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Zintl").content(
                VStack::new((HStack::new((Button::new("Save"), Button::new("Cancel")))
                    .fill_width()
                    .space_between()
                    .equal_width_children(),))
                .fill_width(),
            ),
        );
        let tree = app.render_tree();
        let column = &tree.children[0];
        let row = &column.children[0];

        assert!(matches!(
            column.value,
            RenderNode::NSView {
                layout: LayoutStyle {
                    width: LayoutDimension::Percent(1.0),
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            row.value,
            RenderNode::NSView {
                layout: LayoutStyle {
                    width: LayoutDimension::Percent(1.0),
                    main_axis_distribution: MainAxisDistribution::SpaceBetween,
                    child_sizing: ChildSizing::Equal,
                    ..
                },
                ..
            }
        ));
    }

    #[test]
    fn text_field_writes_native_input_to_its_store() {
        // Verifies a bound TextField reads native input into its Store and rerenders.
        let captured = Rc::new(Cell::new(None));
        let mut app = App::new(StoreTextFieldView {
            value: None,
            captured: captured.clone(),
        });
        assert!(
            matches!(app.render(), RenderNode::NSTextField { value, .. } if value == "initial")
        );
        let store = captured.get().expect("init must publish the test Store");

        assert!(app.dispatch_root_event(Event::TextChanged {
            value: "typed value".into(),
        }));

        let stored = app.composer.context(|cx| cx.get(store).clone());
        assert_eq!(stored, "typed value");
        assert!(matches!(
            app.render(),
            RenderNode::NSTextField { value, .. } if value == "typed value"
        ));
    }

    #[test]
    fn button_dispatches_its_registered_action() {
        // Verifies the desktop Button delegates activation to the AppKit view route.
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
        // Verifies wrapper IDs reach native render nodes for accessibility.
        let app = App::new(
            Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Zintl")
                .id("main-window")
                .content(TextField::new().id("name-input")),
        );
        let tree = app.render_tree();

        assert!(matches!(
            &tree.value,
            RenderNode::NSWindow { id: Some(id), .. } if id == "main-window"
        ));
        assert!(matches!(
            &tree.children[0].value,
            RenderNode::NSTextField { id: Some(id), .. } if id == "name-input"
        ));
    }

    #[test]
    fn store_watcher_rebuilds_only_its_dependent_element() {
        // Verifies wrappers preserve fine-grained Store watcher rebuilding.
        let renders = Rc::new(Cell::new(0));
        let captured = Rc::new(Cell::new(None));
        let mut app = App::new(BoundTextView {
            value: None,
            renders: renders.clone(),
            captured: captured.clone(),
        });
        let tree = app.render_tree();
        assert!(matches!(
            &tree.children[0].value,
            RenderNode::NSTextField { .. }
        ));
        let store = captured.get().expect("init must publish the test Store");

        app.update_text_store(store, "updated".into());

        let tree = app.render_tree();
        let RenderNode::NSTextField { value, .. } = &tree.children[1].value else {
            panic!("the watched Text must remain an NSTextField")
        };
        assert_eq!(value, "Stored value: updated");
        assert_eq!(renders.get(), 1);
    }

    #[test]
    fn sidebar_selection_round_trips_and_rejects_unknown_items() {
        // Verifies the desktop Sidebar preserves AppKit Store selection behavior.
        struct Navigation {
            selection: Store<Option<String>>,
            captured: Rc<Cell<Store<Option<String>>>>,
        }

        impl View for Navigation {
            type Output = RenderNode;

            fn init(&mut self, cx: &mut Context<'_>) {
                self.selection = cx.store(Some("home".into()));
                self.captured.set(self.selection);
            }

            fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = RenderNode> {
                Window::new(Rect::new(0.0, 0.0, 640.0, 480.0), "Navigation").sidebar(
                    Sidebar::new([SidebarSection::new([
                        SidebarItem::new("home", "Home"),
                        SidebarItem::new("settings", "Settings"),
                    ])])
                    .bind(self.selection),
                )
            }
        }

        let captured = Rc::new(Cell::new(Store::default()));
        let mut app = App::new(Navigation {
            selection: Store::default(),
            captured: captured.clone(),
        });
        assert!(app.dispatch_root_event(Event::SidebarSelectionChanged {
            id: "settings".into(),
        }));
        assert_eq!(
            app.composer.context(|cx| cx.get(captured.get()).clone()),
            Some("settings".into())
        );
        assert!(app.dispatch_root_event(Event::SidebarSelectionChanged {
            id: "missing".into(),
        }));
        assert_eq!(
            app.composer.context(|cx| cx.get(captured.get()).clone()),
            Some("settings".into())
        );
    }
}
