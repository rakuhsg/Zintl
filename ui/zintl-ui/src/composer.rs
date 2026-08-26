use crate::element::{Bound, BoundBuilder, Element, ElementKey, IntoElement};
use crate::event::{Event, EventHandlers, EventRouteId, EventRouter};
use crate::hook::HookId;
use crate::renderer::{RenderBackend, RenderNode};
use crate::sequence::Arena;
use crate::view::{Context, InitStores};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};

#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct BoundId {
    slot: u32,
    generation: u32,
}

struct MountPoint<NodeId> {
    parent: NodeId,
    index: usize,
}

struct BoundState<R: RenderNode, NodeId> {
    parent_bound: Option<BoundId>,
    key: Option<ElementKey>,
    builder: Box<dyn BoundBuilder<R>>,
    dependencies: Vec<HookId>,
    children: Vec<MountedElement<R, NodeId>>,
    mount_point: MountPoint<NodeId>,
    init_stores: InitStores,
}

struct BoundSlot<R: RenderNode, NodeId> {
    generation: u32,
    state: Option<BoundState<R, NodeId>>,
}

struct BoundArena<R: RenderNode, NodeId> {
    slots: Vec<BoundSlot<R, NodeId>>,
    free: Vec<u32>,
}

impl<R: RenderNode, NodeId> BoundArena<R, NodeId> {
    fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
        }
    }

    fn allocate(&mut self) -> BoundId {
        if let Some(slot) = self.free.pop() {
            return BoundId {
                slot,
                generation: self.slots[slot as usize].generation,
            };
        }

        let slot = self.slots.len() as u32;
        self.slots.push(BoundSlot {
            generation: 0,
            state: None,
        });
        BoundId {
            slot,
            generation: 0,
        }
    }

    fn get(&self, id: BoundId) -> Option<&BoundState<R, NodeId>> {
        let slot = self.slots.get(id.slot as usize)?;
        (slot.generation == id.generation)
            .then_some(slot.state.as_ref())
            .flatten()
    }

    fn take(&mut self, id: BoundId) -> Option<BoundState<R, NodeId>> {
        let slot = self.slots.get_mut(id.slot as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.state.take()
    }

    fn put(&mut self, id: BoundId, state: BoundState<R, NodeId>) {
        let slot = &mut self.slots[id.slot as usize];
        assert_eq!(slot.generation, id.generation);
        assert!(slot.state.replace(state).is_none());
    }

    fn release(&mut self, id: BoundId) {
        let slot = &mut self.slots[id.slot as usize];
        assert_eq!(slot.generation, id.generation);
        assert!(slot.state.is_none());
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(id.slot);
    }
}

struct MountedNode<R: RenderNode, NodeId> {
    value: R,
    key: Option<ElementKey>,
    handle: NodeId,
    children: Vec<MountedElement<R, NodeId>>,
    event_route: Option<EventRouteId>,
}

enum MountedElement<R: RenderNode, NodeId> {
    Node(MountedNode<R, NodeId>),
    Bound(BoundId),
}

pub struct Composer<R, B>
where
    R: RenderNode,
    B: RenderBackend<R>,
{
    backend: B,
    stores: Arena,
    next_hook_id: u32,
    dirty_hooks: BTreeSet<HookId>,
    subscriptions: Vec<Vec<BoundId>>,
    bounds: BoundArena<R, B::NodeId>,
    root: Vec<MountedElement<R, B::NodeId>>,
    mounted: bool,
    event_router: EventRouter,
}

impl<R, B> Composer<R, B>
where
    R: RenderNode,
    B: RenderBackend<R>,
{
    pub fn new(backend: B) -> Self {
        Self {
            backend,
            stores: Arena::new(),
            next_hook_id: 0,
            dirty_hooks: BTreeSet::new(),
            subscriptions: Vec::new(),
            bounds: BoundArena::new(),
            root: Vec::new(),
            mounted: false,
            event_router: EventRouter::new(),
        }
    }

    pub fn context<T>(&mut self, operation: impl FnOnce(&mut Context<'_>) -> T) -> T {
        let mut context = Context {
            stores: &mut self.stores,
            next_hook_id: &mut self.next_hook_id,
            dirty_hooks: &mut self.dirty_hooks,
            dependencies: None,
            init_stores: None,
        };
        operation(&mut context)
    }

    pub fn mount<E>(&mut self, root: E)
    where
        E: IntoElement<Output = R>,
    {
        assert!(!self.mounted, "a composer can only mount one root");
        self.mounted = true;
        let root_handle = self.backend.root();
        let elements = normalize(vec![root.into_element()]);
        self.root = self.reconcile_list(root_handle, Vec::new(), elements, None, 0);
        let mut root = std::mem::take(&mut self.root);
        self.refresh_mount_points(root_handle, &mut root, None, 0);
        self.root = root;
    }

    pub fn flush(&mut self) {
        let dirty_hooks = std::mem::take(&mut self.dirty_hooks);
        let mut dirty_bounds = HashSet::new();
        for hook in dirty_hooks {
            if let Some(subscribers) = self.subscriptions.get(hook.index()) {
                dirty_bounds.extend(
                    subscribers
                        .iter()
                        .copied()
                        .filter(|id| self.bounds.get(*id).is_some()),
                );
            }
        }

        let mut dirty_bounds: Vec<_> = dirty_bounds.into_iter().collect();
        dirty_bounds.sort_by_key(|id| self.bound_depth(*id));
        let dirty_set: HashSet<_> = dirty_bounds.iter().copied().collect();

        for id in dirty_bounds {
            if self.has_dirty_ancestor(id, &dirty_set) || self.bounds.get(id).is_none() {
                continue;
            }
            self.rebuild_bound(id, None);
        }
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Dispatches an event to the mounted element identified by `route`.
    ///
    /// Stale or unknown routes and events without a matching handler are
    /// ignored. Updates scheduled by a handler are flushed before returning.
    pub fn dispatch_event(&mut self, route: EventRouteId, event: Event) -> bool {
        let handled = {
            let mut context = Context {
                stores: &mut self.stores,
                next_hook_id: &mut self.next_hook_id,
                dirty_hooks: &mut self.dirty_hooks,
                dependencies: None,
                init_stores: None,
            };
            self.event_router.dispatch(route, &mut context, event)
        };
        if handled {
            self.flush();
        }
        handled
    }

    fn bound_depth(&self, mut id: BoundId) -> usize {
        let mut depth = 0;
        while let Some(parent) = self.bounds.get(id).and_then(|state| state.parent_bound) {
            depth += 1;
            id = parent;
        }
        depth
    }

    fn has_dirty_ancestor(&self, id: BoundId, dirty: &HashSet<BoundId>) -> bool {
        let mut parent = self.bounds.get(id).and_then(|state| state.parent_bound);
        while let Some(id) = parent {
            if dirty.contains(&id) {
                return true;
            }
            parent = self.bounds.get(id).and_then(|state| state.parent_bound);
        }
        false
    }

    fn mount_bound(
        &mut self,
        bound: Bound<R>,
        parent: B::NodeId,
        index: usize,
        parent_bound: Option<BoundId>,
    ) -> BoundId {
        let id = self.bounds.allocate();
        self.bounds.put(
            id,
            BoundState {
                parent_bound,
                key: bound.key,
                builder: bound.builder,
                dependencies: Vec::new(),
                children: Vec::new(),
                mount_point: MountPoint { parent, index },
                init_stores: InitStores::new(),
            },
        );
        self.rebuild_bound(id, None);
        id
    }

    fn rebuild_bound(&mut self, id: BoundId, replacement: Option<Bound<R>>) {
        let Some(mut state) = self.bounds.take(id) else {
            return;
        };
        if let Some(replacement) = replacement {
            state.key = replacement.key;
            state.builder = replacement.builder;
        }

        let dependencies = RefCell::new(Vec::new());
        let children = {
            let mut context = Context {
                stores: &mut self.stores,
                next_hook_id: &mut self.next_hook_id,
                dirty_hooks: &mut self.dirty_hooks,
                dependencies: Some(&dependencies),
                init_stores: Some(&mut state.init_stores),
            };
            state.builder.build_children(&mut context)
        };
        let new_dependencies = dependencies.into_inner();

        self.unsubscribe(id, &state.dependencies);
        self.subscribe(id, &new_dependencies);
        state.dependencies = new_dependencies;

        let old_children = std::mem::take(&mut state.children);
        state.children = self.reconcile_list(
            state.mount_point.parent,
            old_children,
            normalize(children),
            Some(id),
            state.mount_point.index,
        );
        self.refresh_mount_points(
            state.mount_point.parent,
            &mut state.children,
            Some(id),
            state.mount_point.index,
        );
        self.bounds.put(id, state);
    }

    fn subscribe(&mut self, id: BoundId, hooks: &[HookId]) {
        for hook in hooks {
            if self.subscriptions.len() <= hook.index() {
                self.subscriptions.resize_with(hook.index() + 1, Vec::new);
            }
            let subscribers = &mut self.subscriptions[hook.index()];
            if !subscribers.contains(&id) {
                subscribers.push(id);
            }
        }
    }

    fn unsubscribe(&mut self, id: BoundId, hooks: &[HookId]) {
        for hook in hooks {
            if let Some(subscribers) = self.subscriptions.get_mut(hook.index()) {
                subscribers.retain(|subscriber| *subscriber != id);
            }
        }
    }

    fn reconcile_list(
        &mut self,
        parent: B::NodeId,
        old: Vec<MountedElement<R, B::NodeId>>,
        new: Vec<Element<R>>,
        parent_bound: Option<BoundId>,
        start_index: usize,
    ) -> Vec<MountedElement<R, B::NodeId>> {
        let mut old: Vec<_> = old.into_iter().map(Some).collect();
        let mut result = Vec::with_capacity(new.len());
        let mut parent_index = start_index;

        for (position, element) in new.into_iter().enumerate() {
            let match_index = self.find_match(&old, &element, position);
            let mounted = if let Some(match_index) = match_index {
                let mounted = old[match_index].take().unwrap();
                self.reconcile_element(parent, parent_index, mounted, element, parent_bound)
            } else {
                self.mount_element(parent, parent_index, element, parent_bound)
            };
            parent_index += self.top_handles(&mounted).len();
            result.push(mounted);
        }

        for mounted in old.into_iter().flatten() {
            self.unmount_element(mounted);
        }

        let mut index = start_index;
        for mounted in &result {
            let handles = self.top_handles(mounted);
            for handle in handles {
                self.backend.move_child(parent, index, handle);
                index += 1;
            }
        }
        result
    }

    fn find_match(
        &self,
        old: &[Option<MountedElement<R, B::NodeId>>],
        new: &Element<R>,
        position: usize,
    ) -> Option<usize> {
        if element_key(new).is_some() {
            return old.iter().position(|candidate| {
                candidate
                    .as_ref()
                    .is_some_and(|mounted| self.elements_match(mounted, new))
            });
        }

        old.get(position)
            .and_then(Option::as_ref)
            .filter(|mounted| self.elements_match(mounted, new))
            .map(|_| position)
    }

    fn elements_match(&self, mounted: &MountedElement<R, B::NodeId>, new: &Element<R>) -> bool {
        match (mounted, new) {
            (MountedElement::Node(old), Element::Node { value, key, .. }) => {
                old.key == *key && old.value.same_kind(value)
            }
            (MountedElement::Bound(id), Element::Bound(new)) => {
                self.bounds.get(*id).is_some_and(|old| {
                    old.key == new.key
                        && old.builder.builder_type_id() == new.builder.builder_type_id()
                })
            }
            _ => false,
        }
    }

    fn mount_element(
        &mut self,
        parent: B::NodeId,
        index: usize,
        element: Element<R>,
        parent_bound: Option<BoundId>,
    ) -> MountedElement<R, B::NodeId> {
        match element {
            Element::Node {
                value,
                key,
                children,
                events,
            } => {
                let event_route = self.mount_event_route(events);
                let handle = self.backend.create(&value, event_route);
                self.backend.insert_child(parent, index, handle);
                let children =
                    self.reconcile_list(handle, Vec::new(), normalize(children), parent_bound, 0);
                MountedElement::Node(MountedNode {
                    value,
                    key,
                    handle,
                    children,
                    event_route,
                })
            }
            Element::Bound(bound) => {
                MountedElement::Bound(self.mount_bound(bound, parent, index, parent_bound))
            }
            Element::Fragment(_) => unreachable!("fragments are normalized before mounting"),
        }
    }

    fn reconcile_element(
        &mut self,
        parent: B::NodeId,
        index: usize,
        mounted: MountedElement<R, B::NodeId>,
        element: Element<R>,
        parent_bound: Option<BoundId>,
    ) -> MountedElement<R, B::NodeId> {
        match (mounted, element) {
            (
                MountedElement::Node(mut old),
                Element::Node {
                    value,
                    key,
                    children,
                    events,
                },
            ) => {
                if old.value != value {
                    self.backend.update(old.handle, &value);
                    old.value = value;
                }
                old.key = key;
                let previous_route = old.event_route;
                old.event_route = self.reconcile_event_route(old.event_route, events);
                if old.event_route != previous_route {
                    self.backend.set_event_route(old.handle, old.event_route);
                }
                old.children = self.reconcile_list(
                    old.handle,
                    old.children,
                    normalize(children),
                    parent_bound,
                    0,
                );
                MountedElement::Node(old)
            }
            (MountedElement::Bound(id), Element::Bound(bound)) => {
                if let Some(state) = self.bounds.get(id) {
                    let old_parent = state.mount_point.parent;
                    let old_index = state.mount_point.index;
                    debug_assert_eq!(old_parent, parent);
                    let _ = old_index;
                }
                if let Some(mut state) = self.bounds.take(id) {
                    state.parent_bound = parent_bound;
                    state.mount_point = MountPoint { parent, index };
                    self.bounds.put(id, state);
                }
                self.rebuild_bound(id, Some(bound));
                MountedElement::Bound(id)
            }
            _ => unreachable!("only matching elements are reconciled"),
        }
    }

    fn unmount_element(&mut self, mounted: MountedElement<R, B::NodeId>) {
        match mounted {
            MountedElement::Node(node) => {
                for child in node.children {
                    self.unmount_element(child);
                }
                if let Some(route) = node.event_route {
                    self.event_router.remove(route);
                }
                self.backend.remove(node.handle);
            }
            MountedElement::Bound(id) => {
                if let Some(state) = self.bounds.take(id) {
                    self.unsubscribe(id, &state.dependencies);
                    for child in state.children {
                        self.unmount_element(child);
                    }
                    self.bounds.release(id);
                }
            }
        }
    }

    fn top_handles(&self, mounted: &MountedElement<R, B::NodeId>) -> Vec<B::NodeId> {
        match mounted {
            MountedElement::Node(node) => vec![node.handle],
            MountedElement::Bound(id) => self
                .bounds
                .get(*id)
                .map(|state| {
                    state
                        .children
                        .iter()
                        .flat_map(|child| self.top_handles(child))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    fn refresh_mount_points(
        &mut self,
        parent: B::NodeId,
        elements: &mut [MountedElement<R, B::NodeId>],
        parent_bound: Option<BoundId>,
        start_index: usize,
    ) {
        let mut index = start_index;
        for element in elements {
            match element {
                MountedElement::Node(node) => {
                    self.refresh_mount_points(node.handle, &mut node.children, parent_bound, 0);
                    index += 1;
                }
                MountedElement::Bound(id) => {
                    let Some(mut state) = self.bounds.take(*id) else {
                        continue;
                    };
                    state.parent_bound = parent_bound;
                    state.mount_point = MountPoint { parent, index };
                    self.refresh_mount_points(parent, &mut state.children, Some(*id), index);
                    index += state
                        .children
                        .iter()
                        .map(|child| self.top_handles(child).len())
                        .sum::<usize>();
                    self.bounds.put(*id, state);
                }
            }
        }
    }

    fn mount_event_route(&mut self, events: EventHandlers) -> Option<EventRouteId> {
        (!events.is_empty()).then(|| self.event_router.insert(events))
    }

    fn reconcile_event_route(
        &mut self,
        current: Option<EventRouteId>,
        events: EventHandlers,
    ) -> Option<EventRouteId> {
        if events.is_empty() {
            if let Some(route) = current {
                self.event_router.remove(route);
            }
            return None;
        }

        if let Some(route) = current {
            assert!(self.event_router.replace(route, events));
            Some(route)
        } else {
            Some(self.event_router.insert(events))
        }
    }
}

fn element_key<R: RenderNode>(element: &Element<R>) -> Option<&ElementKey> {
    match element {
        Element::Node { key, .. } => key.as_ref(),
        Element::Bound(bound) => bound.key.as_ref(),
        Element::Fragment(_) => None,
    }
}

fn normalize<R: RenderNode>(elements: Vec<Element<R>>) -> Vec<Element<R>> {
    let mut normalized = Vec::new();
    for element in elements {
        match element {
            Element::Fragment(children) => normalized.extend(normalize(children)),
            element => normalized.push(element),
        }
    }
    normalized
}
