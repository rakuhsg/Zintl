use crate::event::{Event, EventRouteId};
use crate::view::{Context, StoreContext};
use std::fmt::Debug;

pub trait RenderNode: Clone + PartialEq + 'static {
    type Event: Event;
    type Context<'a>: Context<'a>;

    fn same_kind(&self, other: &Self) -> bool;
}

pub trait RenderBackend<R: RenderNode> {
    type NodeId: Copy + Debug + Eq;

    fn create_context<'a>(&'a self, store_context: StoreContext<'a>) -> R::Context<'a>;
    fn root(&self) -> Self::NodeId;
    /// Creates a backend node and associates its mounted Element route, if any.
    fn create(&mut self, node: &R, event_route: Option<EventRouteId>) -> Self::NodeId;
    fn update(&mut self, node: Self::NodeId, value: &R);
    /// Replaces the route associated with an existing backend node.
    fn set_event_route(&mut self, node: Self::NodeId, event_route: Option<EventRouteId>);
    fn insert_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId);
    fn remove(&mut self, node: Self::NodeId);
    fn move_child(&mut self, parent: Self::NodeId, index: usize, child: Self::NodeId);
}
