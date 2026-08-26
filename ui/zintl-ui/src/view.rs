use crate::element::{Bound, BoundBuilder, Element, IntoElement};
use crate::hook::HookId;
use crate::renderer::RenderNode;
use crate::sequence::Arena;
use crate::store::Store;
use std::any::{TypeId, type_name};
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::marker::PhantomData;

pub struct Context<'a> {
    pub(crate) stores: &'a mut Arena,
    pub(crate) next_hook_id: &'a mut u32,
    pub(crate) dirty_hooks: &'a mut BTreeSet<HookId>,
    pub(crate) dependencies: Option<&'a RefCell<Vec<HookId>>>,
    pub(crate) init_stores: Option<&'a mut InitStores>,
}

pub(crate) struct StoreSlot {
    type_id: TypeId,
    type_name: &'static str,
    store_id: crate::store::StoreId,
    hook_id: HookId,
}

pub(crate) struct InitStores {
    pub(crate) slots: Vec<StoreSlot>,
    next: usize,
}

impl InitStores {
    pub(crate) fn new() -> Self {
        Self {
            slots: Vec::new(),
            next: 0,
        }
    }

    pub(crate) fn begin(&mut self) {
        self.next = 0;
    }
}

impl Context<'_> {
    pub fn store<T: 'static>(&mut self, value: T) -> Store<T> {
        if let Some(init_stores) = &mut self.init_stores {
            let index = init_stores.next;
            init_stores.next += 1;
            if let Some(slot) = init_stores.slots.get(index) {
                assert_eq!(
                    slot.type_id,
                    TypeId::of::<T>(),
                    "View::init store type changed at slot {index}: expected {}, got {}",
                    slot.type_name,
                    type_name::<T>(),
                );
                return Store::new(slot.store_id, slot.hook_id);
            }

            let store_id = self.stores.insert(value);
            let hook_id = HookId::new(*self.next_hook_id);
            *self.next_hook_id += 1;
            init_stores.slots.push(StoreSlot {
                type_id: TypeId::of::<T>(),
                type_name: type_name::<T>(),
                store_id,
                hook_id,
            });
            return Store::new(store_id, hook_id);
        }

        let store_id = self.stores.insert(value);
        let hook_id = HookId::new(*self.next_hook_id);
        *self.next_hook_id += 1;
        Store::new(store_id, hook_id)
    }

    pub fn get<T: 'static>(&self, store: Store<T>) -> &T {
        let handle = store.handle();
        if let Some(dependencies) = self.dependencies {
            let mut dependencies = dependencies.borrow_mut();
            if !dependencies.contains(&handle.hook_id) {
                dependencies.push(handle.hook_id);
            }
        }
        self.stores
            .get(handle.id)
            .expect("store handle must belong to this composer")
    }

    pub fn watch<T, F, E>(&self, store: Store<T>, render: F) -> StoreWatcher<T, F, E>
    where
        T: 'static,
        F: Fn(&T) -> E + 'static,
        E: IntoElement + 'static,
    {
        store.handle();
        StoreWatcher {
            store,
            render,
            element: PhantomData,
        }
    }

    pub fn update<T: 'static, U>(
        &mut self,
        store: Store<T>,
        update: impl FnOnce(&mut T) -> U,
    ) -> U {
        let handle = store.handle();
        let value = self
            .stores
            .get_mut(handle.id)
            .expect("store handle must belong to this composer");
        let result = update(value);
        self.dirty_hooks.insert(handle.hook_id);
        result
    }
}

pub struct StoreWatcher<T: 'static, F, E> {
    store: Store<T>,
    render: F,
    element: PhantomData<fn(&T) -> E>,
}

impl<T: 'static, F: Clone, E> Clone for StoreWatcher<T, F, E> {
    fn clone(&self) -> Self {
        Self {
            store: self.store,
            render: self.render.clone(),
            element: PhantomData,
        }
    }
}

struct StoreWatcherBuilder<T: 'static, F, E> {
    store: Store<T>,
    render: F,
    element: PhantomData<fn(&T) -> E>,
}

impl<T, F, E> BoundBuilder<E::Output> for StoreWatcherBuilder<T, F, E>
where
    T: 'static,
    F: Fn(&T) -> E + 'static,
    E: IntoElement + 'static,
{
    fn build_children(&mut self, cx: &mut Context<'_>) -> Vec<Element<E::Output>> {
        vec![(self.render)(cx.get(self.store)).into_element()]
    }

    fn builder_type_id(&self) -> TypeId {
        TypeId::of::<Self>()
    }
}

impl<T, F, E> IntoElement for StoreWatcher<T, F, E>
where
    T: 'static,
    F: Fn(&T) -> E + 'static,
    E: IntoElement + 'static,
{
    type Output = E::Output;

    fn into_element(self) -> Element<Self::Output> {
        Element::Bound(Bound {
            key: None,
            builder: Box::new(StoreWatcherBuilder {
                store: self.store,
                render: self.render,
                element: PhantomData,
            }),
        })
    }
}

pub trait View: 'static {
    type Output: RenderNode;

    fn init(&mut self, _cx: &mut Context<'_>) {}

    fn render(&self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output>;
}

struct ViewBuilder<V> {
    view: V,
    initialized: bool,
}

impl<V: View> BoundBuilder<V::Output> for ViewBuilder<V> {
    fn build_children(&mut self, cx: &mut Context<'_>) -> Vec<Element<V::Output>> {
        if !self.initialized {
            if let Some(init_stores) = &mut cx.init_stores {
                init_stores.begin();
            }
            self.view.init(cx);
            self.initialized = true;
        }
        vec![self.view.render(cx).into_element()]
    }

    fn builder_type_id(&self) -> TypeId {
        TypeId::of::<V>()
    }
}

impl<V: View> IntoElement for V {
    type Output = V::Output;

    fn into_element(self) -> Element<Self::Output> {
        Element::Bound(Bound {
            key: None,
            builder: Box::new(ViewBuilder {
                view: self,
                initialized: false,
            }),
        })
    }
}
