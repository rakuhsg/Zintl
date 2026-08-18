use std::fmt::Debug;

pub trait RenderNode: Clone + PartialEq + 'static {
    fn same_kind(&self, other: &Self) -> bool;
}

pub trait RenderBackend<R: RenderNode> {
    type NodeId: Copy + Debug + Eq;

    fn root(&self) -> Self::NodeId;
    fn create(&mut self, node: &R) -> Self::NodeId;
    fn update(&mut self, node: Self::NodeId, value: &R);
    fn insert_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId);
    fn remove(&mut self, node: Self::NodeId);
    fn move_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId);
}
