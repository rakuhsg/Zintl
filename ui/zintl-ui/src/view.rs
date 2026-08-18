use crate::element::{Bound, BoundBuilder, Element, IntoElement};
use crate::hook::HookId;
use crate::renderer::RenderNode;
use crate::sequence::Arena;
use crate::store::Store;
use std::any::TypeId;
use std::cell::RefCell;
use std::collections::BTreeSet;

pub struct Context<'a> {
    pub(crate) stores: &'a mut Arena,
    pub(crate) next_hook_id: &'a mut u32,
    pub(crate) dirty_hooks: &'a mut BTreeSet<HookId>,
    pub(crate) dependencies: Option<&'a RefCell<Vec<HookId>>>,
}

impl Context<'_> {
    pub fn store<T: 'static>(&mut self, value: T) -> Store<T> {
        let store_id = self.stores.insert(value);
        let hook_id = HookId::new(*self.next_hook_id);
        *self.next_hook_id += 1;
        Store::new(store_id, hook_id)
    }

    pub fn get<T: 'static>(&self, store: Store<T>) -> &T {
        if let Some(dependencies) = self.dependencies {
            let mut dependencies = dependencies.borrow_mut();
            if !dependencies.contains(&store.hook_id) {
                dependencies.push(store.hook_id);
            }
        }
        self.stores
            .get(store.id)
            .expect("store handle must belong to this composer")
    }

    pub fn update<T: 'static, U>(
        &mut self,
        store: Store<T>,
        update: impl FnOnce(&mut T) -> U,
    ) -> U {
        let value = self
            .stores
            .get_mut(store.id)
            .expect("store handle must belong to this composer");
        let result = update(value);
        self.dirty_hooks.insert(store.hook_id);
        result
    }
}

pub trait View: 'static {
    type Output: RenderNode;

    fn render(&mut self, cx: &mut Context<'_>) -> impl IntoElement<Output = Self::Output>;
}

struct ViewBuilder<V> {
    view: V,
}

impl<V: View> BoundBuilder<V::Output> for ViewBuilder<V> {
    fn build(&mut self, cx: &mut Context<'_>) -> Element<V::Output> {
        self.view.render(cx).into_element()
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
            builder: Box::new(ViewBuilder { view: self }),
        })
    }
}
