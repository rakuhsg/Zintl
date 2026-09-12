use zintl_ui::element::IntoElement;
use zintl_ui::store::Store;
use zintl_ui::view::{Context as ContextTrait, StoreContext, StoreWatcher};

pub type MainTask = Box<dyn FnOnce() + Send + 'static>;

pub type MainTaskSender = dyn Fn(MainTask) -> bool + Send + Sync;

pub struct AppKitContext<'a> {
    store_context: StoreContext<'a>,
    perform_main: Option<&'a MainTaskSender>,
}

impl<'a> AppKitContext<'a> {
    #[doc(hidden)]
    pub fn new(store_context: StoreContext<'a>, perform_main: Option<&'a MainTaskSender>) -> Self {
        Self {
            store_context,
            perform_main,
        }
    }

    /// Queues a task for execution on the platform main thread.
    ///
    /// # Panics
    /// Panics when the active backend has no main-thread sender or its message
    /// loop no longer accepts work.
    pub fn perform_main(&self, task: impl FnOnce() + Send + 'static) {
        let sender = self
            .perform_main
            .expect("the active backend does not provide a main-thread sender");
        assert!(
            sender(Box::new(task)),
            "the main-thread message loop is closed"
        );
    }

    pub fn store<T: 'static>(&mut self, value: T) -> Store<T> {
        self.store_context.store(value)
    }

    pub fn get<T: 'static>(&self, store: Store<T>) -> &T {
        self.store_context.get(store)
    }

    pub fn watch<T, F, E>(&self, store: Store<T>, render: F) -> StoreWatcher<T, F, E>
    where
        T: 'static,
        F: Fn(&T) -> E + 'static,
        E: IntoElement + 'static,
    {
        self.store_context.watch(store, render)
    }

    pub fn update<T: 'static, U>(
        &mut self,
        store: Store<T>,
        update: impl FnOnce(&mut T) -> U,
    ) -> U {
        self.store_context.update(store, update)
    }
}

impl<'a> ContextTrait<'a> for AppKitContext<'a> {
    fn store_context<'context>(&'context self) -> &'context StoreContext<'a> {
        &self.store_context
    }

    fn store_context_mut<'context>(&'context mut self) -> &'context mut StoreContext<'a> {
        &mut self.store_context
    }
}
