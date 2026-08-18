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

    fn create(&mut self, value: &TestRenderNode) -> Self::NodeId {
        let id = self.nodes.len();
        self.nodes.push(Some(NativeNode {
            value: Some(value.clone()),
            parent: None,
            children: Vec::new(),
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
            Element::node(TestRenderNode::Row(self.id))
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
}
