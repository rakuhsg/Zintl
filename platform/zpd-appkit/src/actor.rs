use std::cell::{RefCell, UnsafeCell};
use std::marker::PhantomData;
use std::pin::Pin;
use std::rc::{Rc, Weak};

use crate::native::{self, Id, Strong};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ActorError {
    Dropped,
    NativeReleased,
    InvalidHierarchy,
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
}

#[derive(Clone)]
pub(crate) struct ActorRef {
    inner: Weak<ActorInner>,
    tree: Weak<RefCell<TreeInner>>,
    id: NodeId,
    _main_thread: PhantomData<Rc<()>>,
}

impl ActorRef {
    pub(crate) fn with<R>(&self, operation: impl FnOnce(Id) -> R) -> Result<R, ActorError> {
        let inner = self.inner.upgrade().ok_or(ActorError::Dropped)?;
        let native = inner.native.load().ok_or(ActorError::NativeReleased)?;
        Ok(operation(native.as_ptr()))
    }

    pub(crate) fn is_alive(&self) -> bool {
        self.inner
            .upgrade()
            .is_some_and(|inner| inner.native.load().is_some())
    }

    pub(crate) fn same_tree(&self, other: &Self) -> bool {
        Weak::ptr_eq(&self.tree, &other.tree)
    }

    pub(crate) fn remove(&self) {
        if let Some(tree) = self.tree.upgrade() {
            TreeInner::remove_subtree(&tree, self.id);
        }
    }

    pub(crate) fn reparent_to(&self, parent: &Self) -> Result<(), ActorError> {
        let tree = self.tree.upgrade().ok_or(ActorError::Dropped)?;
        ActorTree { inner: tree }.reparent(self, parent)
    }

    pub(crate) fn move_to_root(&self) -> Result<(), ActorError> {
        let tree = self.tree.upgrade().ok_or(ActorError::Dropped)?;
        ActorTree { inner: tree }.move_to_root(self)
    }

    pub(crate) fn tree_handle(&self) -> Option<ActorTree> {
        self.tree.upgrade().map(|inner| ActorTree { inner })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NodeId {
    index: usize,
    generation: u32,
}

struct Node {
    generation: u32,
    parent: Option<NodeId>,
    children: Vec<NodeId>,
    actor: Rc<ActorInner>,
    attachments: Vec<Strong>,
    _native: Strong,
    root: bool,
}

struct TreeInner {
    nodes: Vec<Option<Node>>,
    generations: Vec<u32>,
    root: NodeId,
}

#[derive(Clone)]
pub(crate) struct ActorTree {
    inner: Rc<RefCell<TreeInner>>,
}

impl ActorTree {
    pub(crate) fn new(root: Strong) -> Self {
        let actor = Rc::new(ActorInner {
            native: WeakSlot::new(root.as_ptr()),
        });
        let root_id = NodeId {
            index: 0,
            generation: 0,
        };
        Self {
            inner: Rc::new(RefCell::new(TreeInner {
                nodes: vec![Some(Node {
                    generation: 0,
                    parent: None,
                    children: Vec::new(),
                    actor,
                    attachments: Vec::new(),
                    _native: root,
                    root: true,
                })],
                generations: vec![0],
                root: root_id,
            })),
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

    pub(crate) fn insert_child(
        &self,
        parent: &ActorRef,
        native: Strong,
    ) -> Result<ActorRef, ActorError> {
        if !Weak::ptr_eq(&parent.tree, &Rc::downgrade(&self.inner)) {
            return Err(ActorError::InvalidHierarchy);
        }
        Ok(self.insert(parent.id, native))
    }

    fn insert(&self, parent: NodeId, native: Strong) -> ActorRef {
        let actor = Rc::new(ActorInner {
            native: WeakSlot::new(native.as_ptr()),
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
        let id = NodeId {
            index,
            generation: tree.generations[index],
        };
        tree.nodes[index] = Some(Node {
            generation: id.generation,
            parent: Some(parent),
            children: Vec::new(),
            actor: actor.clone(),
            attachments: Vec::new(),
            _native: native,
            root: false,
        });
        tree.node_mut(parent)
            .expect("parent must be live")
            .children
            .push(id);
        ActorRef {
            inner: Rc::downgrade(&actor),
            tree: Rc::downgrade(&self.inner),
            id,
            _main_thread: PhantomData,
        }
    }

    pub(crate) fn actor_ref(&self, id: NodeId) -> Option<ActorRef> {
        let tree = self.inner.borrow();
        let node = tree.node(id)?;
        Some(ActorRef {
            inner: Rc::downgrade(&node.actor),
            tree: Rc::downgrade(&self.inner),
            id,
            _main_thread: PhantomData,
        })
    }

    pub(crate) fn attach(&self, owner: &ActorRef, attachment: Strong) -> Result<(), ActorError> {
        let mut tree = self.inner.borrow_mut();
        let node = tree.node_mut(owner.id).ok_or(ActorError::Dropped)?;
        node.attachments.push(attachment);
        Ok(())
    }

    pub(crate) fn reparent(&self, child: &ActorRef, parent: &ActorRef) -> Result<(), ActorError> {
        if !child.same_tree(parent) || child.id == parent.id {
            return Err(ActorError::InvalidHierarchy);
        }
        let mut tree = self.inner.borrow_mut();
        if tree.is_descendant(parent.id, child.id) {
            return Err(ActorError::InvalidHierarchy);
        }
        let old_parent = tree.node(child.id).ok_or(ActorError::Dropped)?.parent;
        tree.node(parent.id).ok_or(ActorError::Dropped)?;
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
    fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes
            .get(id.index)?
            .as_ref()
            .filter(|node| node.generation == id.generation)
    }

    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes
            .get_mut(id.index)?
            .as_mut()
            .filter(|node| node.generation == id.generation)
    }

    fn is_descendant(&self, candidate: NodeId, ancestor: NodeId) -> bool {
        let mut parent = self.node(candidate).and_then(|node| node.parent);
        while let Some(id) = parent {
            if id == ancestor {
                return true;
            }
            parent = self.node(id).and_then(|node| node.parent);
        }
        false
    }

    fn remove_subtree(tree: &Rc<RefCell<Self>>, id: NodeId) {
        let children = {
            let tree = tree.borrow();
            let Some(node) = tree.node(id) else {
                return;
            };
            if node.root {
                return;
            }
            node.children.clone()
        };
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
            }
            tree.generations[id.index] = tree.generations[id.index].wrapping_add(1);
            tree.nodes[id.index].take()
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
        // Verifies subtree teardown invalidates every child ActorRef.
        let tree = ActorTree::new(crate::native::alloc_init(b"NSObject\0"));
        let parent = tree.insert_root(crate::native::alloc_init(b"NSObject\0"));
        let child = tree
            .insert_child(&parent, crate::native::alloc_init(b"NSObject\0"))
            .unwrap();
        parent.remove();
        assert_eq!(child.with(|_| ()), Err(ActorError::Dropped));
    }

    #[test]
    fn weak_slot_observes_native_release() {
        // Verifies an Objective-C object released outside an actor resolves to nil safely.
        let native = crate::native::alloc_init(b"NSObject\0");
        let weak = WeakSlot::new(native.as_ptr());
        drop(native);
        assert!(weak.load().is_none());
    }
}
