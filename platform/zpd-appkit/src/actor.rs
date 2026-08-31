use std::cell::{Cell, RefCell, UnsafeCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::{Rc, Weak};

use crate::native::{self, Id, Strong};

/// Opaque identity of an Actor node within one Application session.
///
/// IDs remain distinct when an Actor slot is reused or a new Application
/// session starts. They do not retain the native object or its Actor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ActorId {
    session: u64,
    index: usize,
    generation: u32,
}

/// Backend-defined event route carried by an Actor without interpreting it.
///
/// The Actor tree only transports this token to its window event callback. Its
/// owner is responsible for validating the token before dispatch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EventRouteToken(u64);

impl EventRouteToken {
    /// Creates a token from a backend-owned representation.
    pub fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the backend-owned representation.
    pub fn get(self) -> u64 {
        self.0
    }
}

/// A semantic event emitted by a Window or one of its attached controls.
///
/// `window` identifies the containing Window and `target` identifies the Actor
/// that emitted the event. For Window lifecycle events both IDs are equal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowEvent {
    pub window: ActorId,
    pub target: ActorId,
    /// The route attached to `target` when the event was emitted.
    pub route: Option<EventRouteToken>,
    pub kind: WindowEventKind,
}

/// Semantic AppKit events supported by the Actor tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowEventKind {
    Created,
    DidResize,
    WillClose,
    DidClose,
    ButtonClicked,
    TextChanged { value: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActorError {
    Dropped,
    NativeReleased,
    NotActive,
    InvalidHierarchy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplicationMessage {
    Run,
    Stop,
}

struct WeakSlot {
    value: UnsafeCell<Id>,
}

impl WeakSlot {
    fn new(value: Id) -> Pin<Box<Self>> {
        let slot = Box::pin(Self {
            value: UnsafeCell::new(std::ptr::null_mut()),
        });
        // SAFETY: The pinned allocation gives Objective-C weak storage a stable address.
        unsafe { native::objc_initWeak(slot.value.get(), value) };
        slot
    }

    fn load(&self) -> Option<Strong> {
        // SAFETY: objc_loadWeakRetained atomically returns a +1 object or nil.
        unsafe { Strong::from_retained(native::objc_loadWeakRetained(self.value.get())) }
    }
}

impl Drop for WeakSlot {
    fn drop(&mut self) {
        // SAFETY: This slot was initialized once and remains at a stable address.
        unsafe { native::objc_destroyWeak(self.value.get()) };
    }
}

struct ActorInner {
    native: Pin<Box<WeakSlot>>,
    alive: Cell<bool>,
}

/// A non-retaining observation of an Objective-C object's lifetime.
///
/// This handle is independent of its Actor node, so it can distinguish an
/// invalidated Actor from a native object that is still retained elsewhere.
pub struct NativeWeakRef {
    native: Pin<Box<WeakSlot>>,
    _main_thread: PhantomData<Rc<()>>,
}

impl NativeWeakRef {
    /// Returns whether the observed Objective-C object has not been deallocated.
    pub fn is_alive(&self) -> bool {
        self.native.load().is_some()
    }
}

#[derive(Clone)]
pub struct ActorRef {
    inner: Weak<ActorInner>,
    tree: Weak<RefCell<TreeInner>>,
    id: NodeKey,
    session: u64,
    _main_thread: PhantomData<Rc<()>>,
}

impl ActorRef {
    /// Returns this Actor's non-retaining identity.
    pub fn actor_id(&self) -> ActorId {
        ActorId {
            session: self.session,
            index: self.id.index,
            generation: self.id.generation,
        }
    }

    /// Replaces the opaque event route associated with this Actor.
    pub fn set_event_route(&self, route: Option<EventRouteToken>) -> Result<(), ActorError> {
        let tree = self.tree.upgrade().ok_or(ActorError::Dropped)?;
        if tree.borrow().active_session != Some(self.session) {
            return Err(ActorError::NotActive);
        }
        tree.borrow_mut()
            .node_mut(self.id)
            .ok_or(ActorError::Dropped)?
            .event_route = route;
        Ok(())
    }

    pub(crate) fn with<R>(&self, operation: impl FnOnce(Id) -> R) -> Result<R, ActorError> {
        let tree = self.tree.upgrade().ok_or(ActorError::Dropped)?;
        if tree.borrow().active_session != Some(self.session) {
            return Err(ActorError::NotActive);
        }
        let inner = self.inner.upgrade().ok_or(ActorError::Dropped)?;
        if !inner.alive.get() {
            return Err(ActorError::Dropped);
        }
        let native = inner.native.load().ok_or(ActorError::NativeReleased)?;
        Ok(operation(native.as_ptr()))
    }

    pub fn is_alive(&self) -> bool {
        self.tree
            .upgrade()
            .is_some_and(|tree| tree.borrow().active_session == Some(self.session))
            && self
                .inner
                .upgrade()
                .is_some_and(|inner| inner.alive.get() && inner.native.load().is_some())
    }

    /// Creates a non-retaining handle that can observe native deallocation.
    ///
    /// # Errors
    /// Returns an error when this Actor or its Application session is no longer active.
    pub fn downgrade_native(&self) -> Result<NativeWeakRef, ActorError> {
        self.with(|native| NativeWeakRef {
            native: WeakSlot::new(native),
            _main_thread: PhantomData,
        })
    }

    pub fn send(&self, message: ApplicationMessage) -> Result<(), ActorError> {
        let tree = self.tree.upgrade().ok_or(ActorError::Dropped)?;
        if tree.borrow().root != self.id {
            return Err(ActorError::InvalidHierarchy);
        }
        crate::runloop::send_application_message(self, message)
    }

    pub(crate) fn same_tree(&self, other: &Self) -> bool {
        self.session == other.session && Weak::ptr_eq(&self.tree, &other.tree)
    }

    pub(crate) fn remove(&self) {
        if let Some(tree) = self.tree.upgrade() {
            TreeInner::remove_subtree(&tree, self.id);
        }
    }

    pub(crate) fn move_to_root(&self) -> Result<(), ActorError> {
        let tree = self.tree.upgrade().ok_or(ActorError::Dropped)?;
        ActorTree {
            inner: tree,
            session: self.session,
        }
        .move_to_root(self)
    }

    pub(crate) fn tree_handle(&self) -> Option<ActorTree> {
        self.tree.upgrade().map(|inner| ActorTree {
            inner,
            session: self.session,
        })
    }
}

impl std::fmt::Display for ActorError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Dropped => "the actor was dropped",
            Self::NativeReleased => "the native object was released",
            Self::NotActive => "the actor session is not active",
            Self::InvalidHierarchy => "the actor hierarchy is invalid",
        })
    }
}

impl std::error::Error for ActorError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodeKey {
    index: usize,
    generation: u32,
}

struct Node {
    generation: u32,
    parent: Option<NodeKey>,
    children: Vec<NodeKey>,
    dependencies: Vec<NodeKey>,
    dependents: Vec<NodeKey>,
    owned: HashMap<&'static str, NodeKey>,
    actor: Rc<ActorInner>,
    teardown: Vec<Box<dyn FnOnce(Id)>>,
    _native: Strong,
    root: bool,
    window: bool,
    event_route: Option<EventRouteToken>,
}

struct EventHandler {
    id: u64,
    callback: Box<dyn FnMut(WindowEvent)>,
}

struct TreeInner {
    nodes: Vec<Option<Node>>,
    generations: Vec<u32>,
    root: NodeKey,
    active_session: Option<u64>,
    next_session: u64,
    event_handler: Option<EventHandler>,
    next_event_handler: u64,
}

#[derive(Clone)]
pub(crate) struct ActorTree {
    inner: Rc<RefCell<TreeInner>>,
    session: u64,
}

impl ActorTree {
    pub(crate) fn new(root: Strong) -> Self {
        let actor = Rc::new(ActorInner {
            native: WeakSlot::new(root.as_ptr()),
            alive: Cell::new(true),
        });
        let root_id = NodeKey {
            index: 0,
            generation: 0,
        };
        Self {
            inner: Rc::new(RefCell::new(TreeInner {
                nodes: vec![Some(Node {
                    generation: 0,
                    parent: None,
                    children: Vec::new(),
                    dependencies: Vec::new(),
                    dependents: Vec::new(),
                    owned: HashMap::new(),
                    actor,
                    teardown: Vec::new(),
                    _native: root,
                    root: true,
                    window: false,
                    event_route: None,
                })],
                generations: vec![0],
                root: root_id,
                active_session: Some(0),
                next_session: 1,
                event_handler: None,
                next_event_handler: 0,
            })),
            session: 0,
        }
    }

    pub(crate) fn begin_session(&self) -> Self {
        self.clear();
        let session = {
            let mut tree = self.inner.borrow_mut();
            let session = tree.next_session;
            tree.next_session = tree.next_session.wrapping_add(1);
            tree.active_session = Some(session);
            session
        };
        Self {
            inner: self.inner.clone(),
            session,
        }
    }

    pub(crate) fn end_session(&self) {
        self.clear();
        let mut tree = self.inner.borrow_mut();
        if tree.active_session == Some(self.session) {
            tree.active_session = None;
            tree.event_handler = None;
        }
    }

    pub(crate) fn root(&self) -> ActorRef {
        self.actor_ref(self.inner.borrow().root)
            .expect("the application root always exists")
    }

    pub(crate) fn insert_root(&self, native: Strong) -> ActorRef {
        let root = self.inner.borrow().root;
        self.insert(root, native)
    }

    pub(crate) fn insert_window(&self, native: Strong) -> ActorRef {
        let actor = self.insert_root(native);
        self.inner
            .borrow_mut()
            .node_mut(actor.id)
            .expect("a newly inserted window must be live")
            .window = true;
        actor
    }

    pub(crate) fn insert_child(
        &self,
        parent: &ActorRef,
        native: Strong,
    ) -> Result<ActorRef, ActorError> {
        if !Weak::ptr_eq(&parent.tree, &Rc::downgrade(&self.inner)) {
            return Err(ActorError::InvalidHierarchy);
        }
        if parent.session != self.session || !parent.is_alive() {
            return Err(ActorError::Dropped);
        }
        Ok(self.insert(parent.id, native))
    }

    fn insert(&self, parent: NodeKey, native: Strong) -> ActorRef {
        let actor = Rc::new(ActorInner {
            native: WeakSlot::new(native.as_ptr()),
            alive: Cell::new(true),
        });
        let mut tree = self.inner.borrow_mut();
        let index = tree
            .nodes
            .iter()
            .position(Option::is_none)
            .unwrap_or(tree.nodes.len());
        if index == tree.nodes.len() {
            tree.nodes.push(None);
            tree.generations.push(0);
        }
        let id = NodeKey {
            index,
            generation: tree.generations[index],
        };
        tree.nodes[index] = Some(Node {
            generation: id.generation,
            parent: Some(parent),
            children: Vec::new(),
            dependencies: Vec::new(),
            dependents: Vec::new(),
            owned: HashMap::new(),
            actor: actor.clone(),
            teardown: Vec::new(),
            _native: native,
            root: false,
            window: false,
            event_route: None,
        });
        tree.node_mut(parent)
            .expect("parent must be live")
            .children
            .push(id);
        ActorRef {
            inner: Rc::downgrade(&actor),
            tree: Rc::downgrade(&self.inner),
            id,
            session: self.session,
            _main_thread: PhantomData,
        }
    }

    fn actor_ref(&self, id: NodeKey) -> Option<ActorRef> {
        let tree = self.inner.borrow();
        let node = tree.node(id)?;
        Some(ActorRef {
            inner: Rc::downgrade(&node.actor),
            tree: Rc::downgrade(&self.inner),
            id,
            session: self.session,
            _main_thread: PhantomData,
        })
    }

    pub(crate) fn emit(&self, target: &ActorRef, kind: WindowEventKind) {
        if target.session != self.session || !target.is_alive() {
            return;
        }
        let event = {
            let tree = self.inner.borrow();
            let mut current = Some(target.id);
            let mut window = None;
            while let Some(id) = current {
                let Some(node) = tree.node(id) else { return };
                if node.window {
                    window = Some(id);
                    break;
                }
                current = node.parent;
            }
            let Some(window) = window else { return };
            WindowEvent {
                window: self.public_id(window),
                target: target.actor_id(),
                route: tree.node(target.id).and_then(|node| node.event_route),
                kind,
            }
        };
        let mut callback = {
            let mut tree = self.inner.borrow_mut();
            tree.event_handler.take()
        };
        if let Some(handler) = callback.as_mut() {
            (handler.callback)(event);
        }
        let mut tree = self.inner.borrow_mut();
        if tree.event_handler.is_none() {
            tree.event_handler = callback;
        }
    }

    pub(crate) fn set_event_handler(
        &self,
        callback: impl FnMut(WindowEvent) + 'static,
    ) -> Result<u64, ActorError> {
        let mut tree = self.inner.borrow_mut();
        if tree.active_session != Some(self.session) {
            return Err(ActorError::NotActive);
        }
        if tree.event_handler.is_some() {
            return Err(ActorError::InvalidHierarchy);
        }
        let id = tree.next_event_handler;
        tree.next_event_handler = tree.next_event_handler.wrapping_add(1);
        tree.event_handler = Some(EventHandler {
            id,
            callback: Box::new(callback),
        });
        Ok(id)
    }

    pub(crate) fn clear_event_handler(&self, id: u64) {
        let mut tree = self.inner.borrow_mut();
        if tree
            .event_handler
            .as_ref()
            .is_some_and(|handler| handler.id == id)
        {
            tree.event_handler = None;
        }
    }

    fn public_id(&self, id: NodeKey) -> ActorId {
        ActorId {
            session: self.session,
            index: id.index,
            generation: id.generation,
        }
    }

    pub(crate) fn add_teardown(
        &self,
        owner: &ActorRef,
        teardown: impl FnOnce(Id) + 'static,
    ) -> Result<(), ActorError> {
        let mut tree = self.inner.borrow_mut();
        tree.node_mut(owner.id)
            .ok_or(ActorError::Dropped)?
            .teardown
            .push(Box::new(teardown));
        Ok(())
    }

    pub(crate) fn add_dependency(
        &self,
        dependent: &ActorRef,
        dependency: &ActorRef,
    ) -> Result<(), ActorError> {
        if !dependent.same_tree(dependency) {
            return Err(ActorError::InvalidHierarchy);
        }
        let mut tree = self.inner.borrow_mut();
        tree.node(dependent.id).ok_or(ActorError::Dropped)?;
        tree.node(dependency.id).ok_or(ActorError::Dropped)?;
        if !tree
            .node(dependent.id)
            .is_some_and(|node| node.dependencies.contains(&dependency.id))
        {
            tree.node_mut(dependent.id)
                .expect("dependent was validated")
                .dependencies
                .push(dependency.id);
            tree.node_mut(dependency.id)
                .expect("dependency was validated")
                .dependents
                .push(dependent.id);
        }
        Ok(())
    }

    pub(crate) fn replace_owned(
        &self,
        owner: &ActorRef,
        slot: &'static str,
        native: Strong,
    ) -> Result<ActorRef, ActorError> {
        self.clear_owned(owner, slot)?;
        let actor = self.insert_child(owner, native)?;
        self.inner
            .borrow_mut()
            .node_mut(owner.id)
            .ok_or(ActorError::Dropped)?
            .owned
            .insert(slot, actor.id);
        Ok(actor)
    }

    pub(crate) fn clear_owned(
        &self,
        owner: &ActorRef,
        slot: &'static str,
    ) -> Result<(), ActorError> {
        let child = self
            .inner
            .borrow_mut()
            .node_mut(owner.id)
            .ok_or(ActorError::Dropped)?
            .owned
            .remove(slot);
        if let Some(child) = child {
            TreeInner::remove_subtree(&self.inner, child);
        }
        Ok(())
    }

    pub(crate) fn reparent(&self, child: &ActorRef, parent: &ActorRef) -> Result<(), ActorError> {
        self.validate_reparent(child, parent)?;
        let mut tree = self.inner.borrow_mut();
        let old_parent = tree.node(child.id).ok_or(ActorError::Dropped)?.parent;
        if let Some(old_parent) = old_parent
            && let Some(node) = tree.node_mut(old_parent)
        {
            node.children.retain(|candidate| *candidate != child.id);
        }
        tree.node_mut(child.id).ok_or(ActorError::Dropped)?.parent = Some(parent.id);
        tree.node_mut(parent.id)
            .ok_or(ActorError::Dropped)?
            .children
            .push(child.id);
        Ok(())
    }

    pub(crate) fn validate_reparent(
        &self,
        child: &ActorRef,
        parent: &ActorRef,
    ) -> Result<(), ActorError> {
        if !child.same_tree(parent) || child.id == parent.id {
            return Err(ActorError::InvalidHierarchy);
        }
        if !child.is_alive() || !parent.is_alive() {
            return Err(ActorError::Dropped);
        }
        let tree = self.inner.borrow();
        if tree.is_descendant(parent.id, child.id) {
            return Err(ActorError::InvalidHierarchy);
        }
        tree.node(child.id).ok_or(ActorError::Dropped)?;
        tree.node(parent.id).ok_or(ActorError::Dropped)?;
        Ok(())
    }

    pub(crate) fn move_to_root(&self, actor: &ActorRef) -> Result<(), ActorError> {
        let root = self.root();
        self.reparent(actor, &root)
    }

    pub(crate) fn clear(&self) {
        let root = self.inner.borrow().root;
        let children = self
            .inner
            .borrow()
            .node(root)
            .expect("root is live")
            .children
            .clone();
        for child in children {
            TreeInner::remove_subtree(&self.inner, child);
        }
    }

    #[cfg(test)]
    pub(crate) fn remove(&self, actor: &ActorRef) {
        TreeInner::remove_subtree(&self.inner, actor.id);
    }
}

impl TreeInner {
    fn node(&self, id: NodeKey) -> Option<&Node> {
        self.nodes
            .get(id.index)?
            .as_ref()
            .filter(|node| node.generation == id.generation)
    }

    fn node_mut(&mut self, id: NodeKey) -> Option<&mut Node> {
        self.nodes
            .get_mut(id.index)?
            .as_mut()
            .filter(|node| node.generation == id.generation)
    }

    fn is_descendant(&self, candidate: NodeKey, ancestor: NodeKey) -> bool {
        let mut parent = self.node(candidate).and_then(|node| node.parent);
        while let Some(id) = parent {
            if id == ancestor {
                return true;
            }
            parent = self.node(id).and_then(|node| node.parent);
        }
        false
    }

    fn remove_subtree(tree: &Rc<RefCell<Self>>, id: NodeKey) {
        let dependents = {
            let tree = tree.borrow();
            let Some(node) = tree.node(id) else {
                return;
            };
            node.dependents.clone()
        };
        for dependent in dependents {
            Self::remove_subtree(tree, dependent);
        }
        let (children, native, teardown) = {
            let mut tree = tree.borrow_mut();
            let Some(node) = tree.node_mut(id) else {
                return;
            };
            if node.root {
                return;
            }
            node.actor.alive.set(false);
            (
                node.children.clone(),
                node._native.as_ptr(),
                std::mem::take(&mut node.teardown),
            )
        };
        for cleanup in teardown.into_iter().rev() {
            cleanup(native);
        }
        for child in children {
            Self::remove_subtree(tree, child);
        }
        let node = {
            let mut tree = tree.borrow_mut();
            let Some(parent) = tree.node(id).and_then(|node| node.parent) else {
                return;
            };
            if let Some(parent) = tree.node_mut(parent) {
                parent.children.retain(|candidate| *candidate != id);
                parent.owned.retain(|_, candidate| *candidate != id);
            }
            let node = tree.nodes[id.index].take();
            if let Some(node) = &node {
                for dependency in &node.dependencies {
                    if let Some(dependency) = tree.node_mut(*dependency) {
                        dependency.dependents.retain(|candidate| *candidate != id);
                    }
                }
            }
            tree.generations[id.index] = tree.generations[id.index].wrapping_add(1);
            node
        };
        drop(node);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generation_rejects_removed_actor_references() {
        // Verifies a removed node invalidates all weak ActorRef handles.
        let object = crate::native::alloc_init(b"NSObject\0");
        let tree = ActorTree::new(object);
        let child = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        assert!(child.is_alive());
        tree.remove(&child);
        assert_eq!(child.with(|_| ()), Err(ActorError::Dropped));
    }

    #[test]
    fn public_ids_distinguish_reused_slots_and_sessions() {
        // Verifies public IDs cannot alias after Actor slot reuse or Application restart.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let first = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        let first_id = first.actor_id();
        tree.remove(&first);
        let second = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        assert_ne!(first_id, second.actor_id());

        let next_session = tree.begin_session();
        let third = next_session.insert_root(crate::native::alloc_init(b"NSObject\0"));
        assert_ne!(second.actor_id(), third.actor_id());
    }

    #[test]
    fn events_resolve_the_containing_window() {
        // Verifies a control event carries both its target and nearest Window IDs.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let window = tree.insert_window(crate::native::alloc_init(b"NSObject\0"));
        let child = tree
            .insert_child(&window, crate::native::alloc_init(b"NSObject\0"))
            .unwrap();
        let route = EventRouteToken::new(42);
        child.set_event_route(Some(route)).unwrap();
        let events = Rc::new(RefCell::new(Vec::new()));
        let received = events.clone();
        let handler = tree
            .set_event_handler(move |event| received.borrow_mut().push(event))
            .unwrap();
        assert_eq!(
            tree.set_event_handler(|_| ()),
            Err(ActorError::InvalidHierarchy)
        );

        tree.emit(&child, WindowEventKind::ButtonClicked);
        child.move_to_root().unwrap();
        tree.emit(&child, WindowEventKind::ButtonClicked);
        tree.clear_event_handler(handler);
        tree.emit(&child, WindowEventKind::ButtonClicked);

        assert_eq!(
            events.borrow().as_slice(),
            &[WindowEvent {
                window: window.actor_id(),
                target: child.actor_id(),
                route: Some(route),
                kind: WindowEventKind::ButtonClicked,
            }]
        );
    }

    #[test]
    fn tree_rejects_cycles() {
        // Verifies strict hierarchy mutation cannot make a parent its own descendant.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let parent = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        let child = tree
            .insert_child(&parent, crate::native::alloc_init(b"NSObject\0"))
            .unwrap();
        assert_eq!(
            tree.reparent(&parent, &child),
            Err(ActorError::InvalidHierarchy)
        );
    }

    #[test]
    fn parent_removal_invalidates_its_descendants() {
        // Verifies removing a native-owned subtree drops its actors and Objective-C objects.
        // SAFETY: NSAutoreleasePool implements alloc/init and drain with these signatures.
        let autorelease_pool = unsafe {
            let allocated = native::send_id(
                native::class(b"NSAutoreleasePool\0"),
                native::sel(b"alloc\0"),
            );
            native::send_id(allocated, native::sel(b"init\0"))
        };
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let parent = tree.insert_root(crate::native::alloc_init(b"NSMutableArray\0"));
        let child = tree
            .insert_child(&parent, crate::native::alloc_init(b"NSObject\0"))
            .unwrap();

        parent
            .with(|parent_native| {
                child.with(|child_native| {
                    // SAFETY: The receiver is a live NSMutableArray and addObject: retains
                    // the live NSObject for the native parent-child ownership relationship.
                    unsafe {
                        native::send_void_id(
                            parent_native,
                            native::sel(b"addObject:\0"),
                            child_native,
                        )
                    };
                })
            })
            .unwrap()
            .unwrap();

        let native_hierarchy_matches = parent
            .with(|parent_native| {
                child.with(|child_native| {
                    // SAFETY: The receiver is a live NSMutableArray and containsObject: accepts
                    // the live NSObject and returns an Objective-C BOOL.
                    unsafe {
                        native::send_bool_id(
                            parent_native,
                            native::sel(b"containsObject:\0"),
                            child_native,
                        )
                    }
                })
            })
            .unwrap()
            .unwrap();
        assert!(native_hierarchy_matches);

        let parent_native = parent.with(WeakSlot::new).unwrap();
        let child_native = child.with(WeakSlot::new).unwrap();
        parent.remove();

        assert_eq!(parent.with(|_| ()), Err(ActorError::Dropped));
        assert_eq!(child.with(|_| ()), Err(ActorError::Dropped));

        // SAFETY: drain consumes the live autorelease pool created at the start of this test.
        unsafe { native::send_void(autorelease_pool, native::sel(b"drain\0")) };
        assert!(parent_native.load().is_none());
        assert!(child_native.load().is_none());
    }

    #[test]
    fn weak_slot_observes_native_release() {
        // Verifies an Objective-C object released outside an actor resolves to nil safely.
        let native = crate::native::alloc_init(b"NSObject\0");
        let weak = WeakSlot::new(native.as_ptr());
        drop(native);
        assert!(weak.load().is_none());
    }

    #[test]
    fn application_session_actor_does_not_revive() {
        // Verifies an NSApp-style root ActorRef stays invalid after a later session begins.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let first = tree.begin_session();
        let stale = first.root();
        first.end_session();
        assert_eq!(stale.with(|_| ()), Err(ActorError::NotActive));

        let second = tree.begin_session();
        assert!(second.root().is_alive());
        assert_eq!(stale.with(|_| ()), Err(ActorError::NotActive));
    }

    #[test]
    fn replacing_owned_child_releases_the_previous_object() {
        // Verifies named Actor ownership replaces and releases native attachments exactly once.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let owner = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        let first = crate::native::alloc_init(b"NSObject\0");
        let first_weak = WeakSlot::new(first.as_ptr());
        tree.replace_owned(&owner, "delegate", first).unwrap();
        let second = crate::native::alloc_init(b"NSObject\0");
        let second_weak = WeakSlot::new(second.as_ptr());
        tree.replace_owned(&owner, "delegate", second).unwrap();
        assert!(first_weak.load().is_none());
        tree.clear_owned(&owner, "delegate").unwrap();
        assert!(second_weak.load().is_none());
    }

    #[test]
    fn dependency_removal_invalidates_dependent_actor() {
        // Verifies removing a referenced view also removes its constraint-like dependent Actor.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let dependency = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        let dependent = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        tree.add_dependency(&dependent, &dependency).unwrap();
        dependency.remove();
        assert_eq!(dependent.with(|_| ()), Err(ActorError::Dropped));
    }
}
