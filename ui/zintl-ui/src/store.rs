use crate::hook::{Hook, HookId};
use std::marker::PhantomData;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StoreId {
    pub(crate) sequence_id: usize,
    pub(crate) index: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Store<T: 'static> {
    pub(crate) id: StoreId,
    pub(crate) hook_id: HookId,
    phantom: PhantomData<fn() -> T>,
}

impl<T: 'static> Copy for Store<T> {}

impl<T: 'static> Clone for Store<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: 'static> Store<T> {
    pub(crate) fn new(id: StoreId, hook_id: HookId) -> Self {
        Self {
            id,
            hook_id,
            phantom: PhantomData,
        }
    }
}

impl<T: 'static> Hook for Store<T> {
    fn hook_id(&self) -> HookId {
        self.hook_id
    }
}
