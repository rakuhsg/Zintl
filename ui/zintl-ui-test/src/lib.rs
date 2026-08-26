use zintl_ui::event::EventRouteId;
use zintl_ui::renderer::{RenderBackend, RenderNode};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TestRenderNode {
    Container(&'static str),
    Text(String),
    Row(u64),
}

impl RenderNode for TestRenderNode {
    fn same_kind(&self, other: &Self) -> bool {
        matches!(
            (self, other),
            (Self::Container(_), Self::Container(_))
                | (Self::Text(_), Self::Text(_))
                | (Self::Row(_), Self::Row(_))
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Operation {
    Create {
        node: usize,
        value: TestRenderNode,
    },
    Update {
        node: usize,
        value: TestRenderNode,
    },
    Insert {
        parent: usize,
        index: usize,
        child: usize,
    },
    Remove {
        node: usize,
    },
    Move {
        parent: usize,
        index: usize,
        child: usize,
    },
    SetEventRoute {
        node: usize,
        route: Option<EventRouteId>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestTree {
    pub value: TestRenderNode,
    pub children: Vec<TestTree>,
}

struct NativeNode {
    value: Option<TestRenderNode>,
    parent: Option<usize>,
    children: Vec<usize>,
    event_route: Option<EventRouteId>,
}

pub struct TestBackend {
    nodes: Vec<Option<NativeNode>>,
    operations: Vec<Operation>,
}

impl TestBackend {
    pub fn new() -> Self {
        Self {
            nodes: vec![Some(NativeNode {
                value: None,
                parent: None,
                children: Vec::new(),
                event_route: None,
            })],
            operations: Vec::new(),
        }
    }

    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    pub fn clear_operations(&mut self) {
        self.operations.clear();
    }

    pub fn event_route(&self, node: usize) -> Option<EventRouteId> {
        self.node(node).event_route
    }

    pub fn roots(&self) -> Vec<TestTree> {
        self.node(0)
            .children
            .iter()
            .map(|child| self.snapshot(*child))
            .collect()
    }

    fn node(&self, id: usize) -> &NativeNode {
        self.nodes[id].as_ref().expect("node must be mounted")
    }

    fn node_mut(&mut self, id: usize) -> &mut NativeNode {
        self.nodes[id].as_mut().expect("node must be mounted")
    }

    fn detach(&mut self, child: usize) -> Option<(usize, usize)> {
        let parent = self.node(child).parent?;
        let index = self
            .node(parent)
            .children
            .iter()
            .position(|candidate| *candidate == child)
            .unwrap();
        self.node_mut(parent).children.remove(index);
        self.node_mut(child).parent = None;
        Some((parent, index))
    }

    fn snapshot(&self, id: usize) -> TestTree {
        let node = self.node(id);
        TestTree {
            value: node.value.clone().unwrap(),
            children: node
                .children
                .iter()
                .map(|child| self.snapshot(*child))
                .collect(),
        }
    }
}

impl Default for TestBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderBackend<TestRenderNode> for TestBackend {
    type NodeId = usize;

    fn root(&self) -> Self::NodeId {
        0
    }

    fn create(
        &mut self,
        value: &TestRenderNode,
        event_route: Option<EventRouteId>,
    ) -> Self::NodeId {
        let id = self.nodes.len();
        self.nodes.push(Some(NativeNode {
            value: Some(value.clone()),
            parent: None,
            children: Vec::new(),
            event_route,
        }));
        self.operations.push(Operation::Create {
            node: id,
            value: value.clone(),
        });
        id
    }

    fn update(&mut self, node: Self::NodeId, value: &TestRenderNode) {
        self.node_mut(node).value = Some(value.clone());
        self.operations.push(Operation::Update {
            node,
            value: value.clone(),
        });
    }

    fn set_event_route(&mut self, node: Self::NodeId, route: Option<EventRouteId>) {
        self.node_mut(node).event_route = route;
        self.operations
            .push(Operation::SetEventRoute { node, route });
    }

    fn insert_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
        self.detach(child);
        let index = index.min(self.node(parent).children.len());
        self.node_mut(parent).children.insert(index, child);
        self.node_mut(child).parent = Some(parent);
        self.operations.push(Operation::Insert {
            parent,
            index,
            child,
        });
    }

    fn remove(&mut self, node: Self::NodeId) {
        assert!(self.node(node).children.is_empty());
        self.detach(node);
        self.nodes[node] = None;
        self.operations.push(Operation::Remove { node });
    }

    fn move_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId) {
        if self.node(child).parent == Some(parent)
            && self.node(parent).children.get(index) == Some(&child)
        {
            return;
        }
        self.detach(child);
        let index = index.min(self.node(parent).children.len());
        self.node_mut(parent).children.insert(index, child);
        self.node_mut(child).parent = Some(parent);
        self.operations.push(Operation::Move {
            parent,
            index,
            child,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;
    use zintl_ui::composer::Composer;
    use zintl_ui::element::{Element, IntoElement, KeyedElement};
    use zintl_ui::event::{Event, EventKind};
    use zintl_ui::store::Store;
    use zintl_ui::view::{Context, View};

    struct InitializedView {
        count: Option<Store<i32>>,
        exposed_count: Rc<Cell<Option<Store<i32>>>>,
        initializations: Rc<Cell<usize>>,
    }

    impl View for InitializedView {
        type Output = TestRenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            self.initializations.set(self.initializations.get() + 1);
            let count = cx.store(0_i32);
            self.count = Some(count);
            self.exposed_count.set(Some(count));
        }

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            let count = self.count.expect("View::init must run before render");
            Element::node(TestRenderNode::Text(cx.get(count).to_string()))
        }
    }

    #[test]
    fn registers_a_store_during_view_initialization() {
        // View::init creates Store handles before immutable rendering begins.
        let mut composer = Composer::new(TestBackend::new());
        let exposed_count = Rc::new(Cell::new(None));
        let initializations = Rc::new(Cell::new(0));
        composer.mount(InitializedView {
            count: None,
            exposed_count: exposed_count.clone(),
            initializations: initializations.clone(),
        });
        let count = exposed_count.get().unwrap();

        composer.context(|cx| cx.update(count, |value| *value = 7));
        composer.flush();

        assert_eq!(initializations.get(), 1);
        assert_eq!(
            composer.backend().roots(),
            vec![TestTree {
                value: TestRenderNode::Text("7".into()),
                children: vec![],
            }]
        );
    }

    struct StatefulChildView {
        count: Option<Store<i32>>,
        exposed_count: Rc<Cell<Option<Store<i32>>>>,
        renders: Rc<Cell<usize>>,
    }

    struct TextView {
        content: String,
    }

    impl View for TextView {
        type Output = TestRenderNode;

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            Element::node(TestRenderNode::Text(self.content.clone()))
        }
    }

    impl View for StatefulChildView {
        type Output = TestRenderNode;

        fn init(&mut self, cx: &mut Context<'_>) {
            let count = cx.store(0_i32);
            self.count = Some(count);
            self.exposed_count.set(Some(count));
        }

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.renders.set(self.renders.get() + 1);
            let count = self.count.expect("View::init must run before render");
            TextView {
                content: cx.get(count).to_string(),
            }
        }
    }

    struct ParentWithStatefulChild {
        /// Triggers a parent rebuild without contributing to the rendered output.
        version: Store<u32>,
        /// Exposes the child-owned Store so the test can update its state.
        exposed_child_count: Rc<Cell<Option<Store<i32>>>>,
        /// Counts parent renders to verify that child updates remain isolated.
        parent_renders: Rc<Cell<usize>>,
        /// Counts child renders across both child and parent updates.
        child_renders: Rc<Cell<usize>>,
    }

    impl View for ParentWithStatefulChild {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.parent_renders.set(self.parent_renders.get() + 1);
            let _version = cx.get(self.version);
            StatefulChildView {
                count: None,
                exposed_count: self.exposed_child_count.clone(),
                renders: self.child_renders.clone(),
            }
        }
    }

    #[test]
    fn preserves_child_view_state_across_parent_rebuilds() {
        // A child-owned Store updates independently and survives reconstruction of its parent.
        let mut composer = Composer::new(TestBackend::new());
        let version = composer.context(|cx| cx.store(0_u32));
        let exposed_child_count = Rc::new(Cell::new(None));
        let parent_renders = Rc::new(Cell::new(0));
        let child_renders = Rc::new(Cell::new(0));
        composer.mount(ParentWithStatefulChild {
            version,
            exposed_child_count: exposed_child_count.clone(),
            parent_renders: parent_renders.clone(),
            child_renders: child_renders.clone(),
        });
        let child_count = exposed_child_count.get().unwrap();

        composer.context(|cx| cx.update(child_count, |value| *value = 7));
        composer.flush();
        assert_eq!(parent_renders.get(), 1);
        assert_eq!(child_renders.get(), 2);

        composer.context(|cx| cx.update(version, |value| *value += 1));
        composer.flush();

        assert_eq!(parent_renders.get(), 2);
        assert_eq!(child_renders.get(), 3);
        assert_eq!(
            composer.backend().roots(),
            vec![TestTree {
                value: TestRenderNode::Text("7".into()),
                children: vec![],
            }]
        );
    }

    struct CounterView {
        count: Store<i32>,
        renders: Rc<Cell<usize>>,
    }

    impl View for CounterView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.renders.set(self.renders.get() + 1);
            Element::node(TestRenderNode::Text(cx.get(self.count).to_string()))
        }
    }

    struct StaticView {
        renders: Rc<Cell<usize>>,
    }

    impl View for StaticView {
        type Output = TestRenderNode;

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.renders.set(self.renders.get() + 1);
            Element::node(TestRenderNode::Text("static".into()))
        }
    }

    struct RootView {
        count: Store<i32>,
        root_renders: Rc<Cell<usize>>,
        counter_renders: Rc<Cell<usize>>,
        static_renders: Rc<Cell<usize>>,
    }

    impl View for RootView {
        type Output = TestRenderNode;

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.root_renders.set(self.root_renders.get() + 1);
            Element::node(TestRenderNode::Container("root")).with_children([
                CounterView {
                    count: self.count,
                    renders: self.counter_renders.clone(),
                }
                .into_element(),
                StaticView {
                    renders: self.static_renders.clone(),
                }
                .into_element(),
            ])
        }
    }

    #[test]
    fn rebuilds_only_the_bound_that_depends_on_the_updated_store() {
        // A store update rerenders its dependent bound and patches only its native node.
        let mut composer = Composer::new(TestBackend::new());
        let count = composer.context(|cx| cx.store(0_i32));
        let root_renders = Rc::new(Cell::new(0));
        let counter_renders = Rc::new(Cell::new(0));
        let static_renders = Rc::new(Cell::new(0));
        composer.mount(RootView {
            count,
            root_renders: root_renders.clone(),
            counter_renders: counter_renders.clone(),
            static_renders: static_renders.clone(),
        });
        composer.backend_mut().clear_operations();

        composer.context(|cx| cx.update(count, |value| *value = 1));
        composer.flush();

        assert_eq!(root_renders.get(), 1);
        assert_eq!(counter_renders.get(), 2);
        assert_eq!(static_renders.get(), 1);
        assert!(matches!(
            composer.backend().operations(),
            [Operation::Update {
                value: TestRenderNode::Text(value),
                ..
            }] if value == "1"
        ));
    }

    struct ConditionalView {
        visible: Store<bool>,
    }

    impl View for ConditionalView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            if *cx.get(self.visible) {
                Element::fragment([Element::node(TestRenderNode::Text("visible".into()))])
            } else {
                Element::fragment([])
            }
        }
    }

    #[test]
    fn inserts_and_removes_nodes_from_a_bound_fragment() {
        // A conditional bound applies structural patches without rebuilding an unrelated root.
        let mut composer = Composer::new(TestBackend::new());
        let visible = composer.context(|cx| cx.store(true));
        composer.mount(ConditionalView { visible });
        composer.backend_mut().clear_operations();

        composer.context(|cx| cx.update(visible, |value| *value = false));
        composer.flush();
        assert_eq!(composer.backend().roots(), Vec::<TestTree>::new());
        assert!(matches!(
            composer.backend().operations(),
            [Operation::Remove { .. }]
        ));

        composer.backend_mut().clear_operations();
        composer.context(|cx| cx.update(visible, |value| *value = true));
        composer.flush();
        assert_eq!(
            composer.backend().roots(),
            vec![TestTree {
                value: TestRenderNode::Text("visible".into()),
                children: vec![],
            }]
        );
        assert!(matches!(
            composer.backend().operations(),
            [Operation::Create { .. }, Operation::Insert { .. }]
        ));
    }

    struct RowView {
        id: u64,
    }

    impl View for RowView {
        type Output = TestRenderNode;

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            Element::node(TestRenderNode::Row(self.id)).on_event(EventKind::Activated, |_, _| {})
        }
    }

    struct ListView {
        order: Store<Vec<u64>>,
    }

    impl View for ListView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            let rows = cx
                .get(self.order)
                .iter()
                .copied()
                .map(|id| RowView { id }.key(id).into_element())
                .collect::<Vec<_>>();
            Element::node(TestRenderNode::Container("list")).with_children(rows)
        }
    }

    #[test]
    fn reorders_keyed_bounds_without_recreating_native_nodes() {
        // Keys preserve bound and native identities when a dynamic list is reordered.
        let mut composer = Composer::new(TestBackend::new());
        let order = composer.context(|cx| cx.store(vec![1_u64, 2, 3]));
        composer.mount(ListView { order });
        let list = composer.backend().node(0).children[0];
        let routes_before = composer
            .backend()
            .node(list)
            .children
            .iter()
            .map(|node| {
                (
                    composer.backend().node(*node).value.clone().unwrap(),
                    composer.backend().event_route(*node).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        composer.backend_mut().clear_operations();

        composer.context(|cx| cx.update(order, |items| *items = vec![3, 1, 2]));
        composer.flush();

        let operations = composer.backend().operations();
        assert!(
            operations
                .iter()
                .any(|operation| matches!(operation, Operation::Move { .. }))
        );
        assert!(!operations.iter().any(|operation| matches!(
            operation,
            Operation::Create { .. } | Operation::Remove { .. }
        )));
        let routes_after = composer
            .backend()
            .node(list)
            .children
            .iter()
            .map(|node| {
                (
                    composer.backend().node(*node).value.clone().unwrap(),
                    composer.backend().event_route(*node).unwrap(),
                )
            })
            .collect::<Vec<_>>();
        for (value, route) in routes_before {
            assert_eq!(
                routes_after
                    .iter()
                    .find_map(|(candidate, current)| (candidate == &value).then_some(*current)),
                Some(route)
            );
        }
        assert_eq!(
            composer.backend().roots()[0].children,
            vec![
                TestTree {
                    value: TestRenderNode::Row(3),
                    children: vec![],
                },
                TestTree {
                    value: TestRenderNode::Row(1),
                    children: vec![],
                },
                TestTree {
                    value: TestRenderNode::Row(2),
                    children: vec![],
                },
            ]
        );
    }

    struct SwitchingView {
        use_first: Store<bool>,
        first: Store<String>,
        second: Store<String>,
        renders: Rc<Cell<usize>>,
    }

    impl View for SwitchingView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.renders.set(self.renders.get() + 1);
            let value = if *cx.get(self.use_first) {
                cx.get(self.first)
            } else {
                cx.get(self.second)
            };
            Element::node(TestRenderNode::Text(value.clone()))
        }
    }

    #[test]
    fn replaces_dynamic_store_dependencies_after_rebuilding() {
        // Rebuilding a bound unsubscribes dependencies that are no longer read.
        let mut composer = Composer::new(TestBackend::new());
        let (use_first, first, second) = composer.context(|cx| {
            (
                cx.store(true),
                cx.store(String::from("first")),
                cx.store(String::from("second")),
            )
        });
        let renders = Rc::new(Cell::new(0));
        composer.mount(SwitchingView {
            use_first,
            first,
            second,
            renders: renders.clone(),
        });

        composer.context(|cx| cx.update(second, |value| value.push('!')));
        composer.flush();
        assert_eq!(renders.get(), 1);

        composer.context(|cx| cx.update(use_first, |value| *value = false));
        composer.flush();
        assert_eq!(renders.get(), 2);

        composer.context(|cx| cx.update(first, |value| value.push('!')));
        composer.flush();
        assert_eq!(renders.get(), 2);
    }

    struct RoutedView {
        version: Store<u32>,
        result: Store<u32>,
    }

    impl View for RoutedView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            let captured = *cx.get(self.version);
            let result = self.result;
            Element::node(TestRenderNode::Container("routed")).on_event(
                EventKind::Activated,
                move |cx, _| {
                    cx.update(result, |value| *value = captured);
                },
            )
        }
    }

    #[test]
    fn preserves_routes_and_replaces_handlers_during_reconciliation() {
        // An equal render node keeps its route while receiving the newest captured handler state.
        let mut composer = Composer::new(TestBackend::new());
        let (version, result) = composer.context(|cx| (cx.store(1_u32), cx.store(0_u32)));
        composer.mount(RoutedView { version, result });
        let node = composer.backend().node(0).children[0];
        let route = composer.backend().event_route(node).unwrap();
        composer.backend_mut().clear_operations();

        composer.context(|cx| cx.update(version, |value| *value = 2));
        composer.flush();

        assert_eq!(composer.backend().event_route(node), Some(route));
        assert!(
            !composer
                .backend()
                .operations()
                .iter()
                .any(|operation| matches!(operation, Operation::SetEventRoute { .. }))
        );
        assert!(composer.dispatch_event(route, Event::Activated));
        assert_eq!(composer.context(|cx| *cx.get(result)), 2);
    }

    struct ConditionalRouteView {
        visible: Store<bool>,
    }

    impl View for ConditionalRouteView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            if *cx.get(self.visible) {
                Element::node(TestRenderNode::Container("route"))
                    .on_event(EventKind::Activated, |_, _| {})
            } else {
                Element::fragment([])
            }
        }
    }

    #[test]
    fn invalidates_routes_when_elements_unmount() {
        // Unmounting invalidates the old generation even after its route slot is reused.
        let mut composer = Composer::new(TestBackend::new());
        let visible = composer.context(|cx| cx.store(true));
        composer.mount(ConditionalRouteView { visible });
        let node = composer.backend().node(0).children[0];
        let stale = composer.backend().event_route(node).unwrap();

        composer.context(|cx| cx.update(visible, |value| *value = false));
        composer.flush();
        assert!(!composer.dispatch_event(stale, Event::Activated));

        composer.context(|cx| cx.update(visible, |value| *value = true));
        composer.flush();
        let replacement = composer
            .backend()
            .event_route(composer.backend().node(0).children[0])
            .unwrap();
        assert_ne!(replacement, stale);
    }

    struct ToggleHandlerView {
        enabled: Store<bool>,
    }

    impl View for ToggleHandlerView {
        type Output = TestRenderNode;

        fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            let element = Element::node(TestRenderNode::Container("toggle"));
            if *cx.get(self.enabled) {
                element.on_event(EventKind::Activated, |_, _| {})
            } else {
                element
            }
        }
    }

    #[test]
    fn adds_and_removes_routes_when_element_handlers_change() {
        // Reconciliation patches a stable backend node when its handler set becomes nonempty or empty.
        let mut composer = Composer::new(TestBackend::new());
        let enabled = composer.context(|cx| cx.store(false));
        composer.mount(ToggleHandlerView { enabled });
        let node = composer.backend().node(0).children[0];
        assert_eq!(composer.backend().event_route(node), None);

        composer.backend_mut().clear_operations();
        composer.context(|cx| cx.update(enabled, |value| *value = true));
        composer.flush();
        let route = composer.backend().event_route(node).unwrap();
        assert!(matches!(
            composer.backend().operations(),
            [Operation::SetEventRoute {
                node: changed,
                route: Some(_),
            }] if *changed == node
        ));

        composer.backend_mut().clear_operations();
        composer.context(|cx| cx.update(enabled, |value| *value = false));
        composer.flush();
        assert_eq!(composer.backend().event_route(node), None);
        assert!(!composer.dispatch_event(route, Event::Activated));
        assert!(matches!(
            composer.backend().operations(),
            [Operation::SetEventRoute {
                node: changed,
                route: None,
            }] if *changed == node
        ));
    }

    struct HandlerReadView {
        observed: Store<u32>,
        renders: Rc<Cell<usize>>,
    }

    impl View for HandlerReadView {
        type Output = TestRenderNode;

        fn render(&self, _cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output> {
            self.renders.set(self.renders.get() + 1);
            let observed = self.observed;
            Element::node(TestRenderNode::Container("reader")).on_event(
                EventKind::Activated,
                move |cx, _| {
                    let _ = cx.get(observed);
                },
            )
        }
    }

    #[test]
    fn event_handler_reads_do_not_create_render_dependencies() {
        // Reading a Store during dispatch does not subscribe the rendering View to that Store.
        let mut composer = Composer::new(TestBackend::new());
        let observed = composer.context(|cx| cx.store(0_u32));
        let renders = Rc::new(Cell::new(0));
        composer.mount(HandlerReadView {
            observed,
            renders: renders.clone(),
        });
        let node = composer.backend().node(0).children[0];
        let route = composer.backend().event_route(node).unwrap();
        assert!(composer.dispatch_event(route, Event::Activated));

        composer.context(|cx| cx.update(observed, |value| *value += 1));
        composer.flush();
        assert_eq!(renders.get(), 1);
    }
}
