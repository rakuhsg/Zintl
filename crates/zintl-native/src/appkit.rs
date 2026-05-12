use std::ffi::c_void;
use std::sync::{Arc, RwLock};

use crossbeam::queue::SegQueue;

use crate::actor::*;
use crate::messageloop::{Context, MainTask, MessageHandler};

mod ffi;

pub struct AppkitContext<M, H: MessageHandler<M>> {
    mesloop: Arc<AppkitMessageLoop<M, H>>,
}

impl<M, H: MessageHandler<M>> AppkitContext<M, H> {
    pub(crate) fn new(mesloop: Arc<AppkitMessageLoop<M, H>>) -> Self {
        AppkitContext { mesloop }
    }
}

impl<M, H: MessageHandler<M>> Context<M> for AppkitContext<M, H> {
    fn perform_main(&self, f: impl Fn(MainMarker) -> () + 'static, send_after: Option<M>) {}
}

pub struct AppkitMessageLoop<M, H: MessageHandler<M>> {
    initialized: bool,
    handler: RwLock<H>,
    queue: SegQueue<MainTask<M>>,
    phantom: std::marker::PhantomData<M>,
}

impl<M, H: MessageHandler<M>> AppkitMessageLoop<M, H> {
    extern "C" fn cb_perform(s_ptr: *const c_void) {
        let mesloop = unsafe { Arc::from_raw(s_ptr as *mut Self) };
        if let Some(task) = mesloop.queue.pop() {
            (task.f)(MainMarker::new());
        }
    }

    extern "C" fn cb_app_on_init(p_ud: *const c_void) {
        let mesloop = unsafe { Arc::from_raw(p_ud as *mut Self) };
        let cx = mesloop.clone().context();
        //TODO: unwrap
        let mut handler = mesloop.handler.write().unwrap();
        handler.on_init(cx);
    }
    extern "C" fn cb_app_will_terminate(_p_ud: *const c_void) {}

    pub fn new(handler: H) -> Arc<Self> {
        let queue = SegQueue::new();
        let mesloop = Arc::new(AppkitMessageLoop {
            initialized: true,
            handler: handler.into(),
            queue,
            phantom: std::marker::PhantomData,
        });

        let p_ud = Arc::into_raw(mesloop.clone());
        let cb = ffi::AppCallback {
            on_init: Self::cb_app_on_init,
            perform: Self::cb_perform,
            will_terminate: Self::cb_app_will_terminate,
        };
        // SAFETY:
        unsafe { ffi::zintlappkit_init(p_ud as *const c_void, &cb) };

        mesloop
    }

    fn context(self: Arc<Self>) -> AppkitContext<M, H> {
        AppkitContext::new(self.clone())
    }

    pub fn run(&self) {
        if self.initialized {
            // SAFETY: Appkit app is initialized.
            unsafe {
                ffi::zintlappkit_run();
            }
        }
    }
}
