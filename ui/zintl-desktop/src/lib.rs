use zintl_ui::composer::Composer;
pub use zintl_ui::element::{Element, IntoElement};
use zintl_ui::renderer::{RenderBackend, RenderNode as RenderNodeTrait};
pub use zintl_ui::view::{Context, View};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RenderNode {
    Text(String),
}

impl RenderNodeTrait for RenderNode {
    fn same_kind(&self, other: &Self) -> bool {
        matches!((self, other), (Self::Text(_), Self::Text(_)))
    }
}

pub struct Text {
    content: String,
}

impl Text {
    pub fn new(content: String) -> Self {
        Text { content }
    }
}

impl IntoElement for Text {
    type Output = RenderNode;

    fn into_element(self) -> Element<Self::Output> {
        Element::node(RenderNode::Text(self.content))
    }
}

struct Node {
    value: Option<RenderNode>,
    parent: Option<usize>,
    children: Vec<usize>,
}

struct TreeBackend {
    nodes: Vec<Option<Node>>,
}

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

pub struct App {
    composer: Composer<RenderNode, TreeBackend>,
}

impl App {
    pub fn new<E>(root: E) -> Self
    where
        E: IntoElement<Output = RenderNode>,
    {
        let mut composer = Composer::new(TreeBackend::new());
        composer.mount(root);
        Self { composer }
    }

    pub fn render(&self) -> RenderNode {
        let backend = self.composer.backend();
        let root = backend.node(0).children[0];
        backend.node(root).value.clone().unwrap()
    }
}
