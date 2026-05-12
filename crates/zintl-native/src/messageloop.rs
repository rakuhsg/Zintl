use std::sync::Arc;

use crate::actor::{MainActor, MainMarker};

pub struct MainTask<M> {
    pub(crate) f: Box<dyn FnOnce(MainMarker) -> ()>,
    pub(crate) send_after: Option<M>,
}

pub trait Context<M> {
    fn perform_main(&self, f: impl FnOnce(MainMarker) -> () + 'static, send_after: Option<M>);
    fn window_manager(&self) -> WindowManager;
}

#[derive(Clone)]
pub struct WindowManager {
    backend: Arc<dyn WindowManagerBackend>,
}

impl WindowManager {
    pub(crate) fn new(backend: Arc<dyn WindowManagerBackend>) -> Self {
        WindowManager { backend }
    }

    pub fn create_window(&self, marker: MainMarker) -> MainActor<Window> {
        self.backend.create_window(marker)
    }
}

pub(crate) trait WindowManagerBackend {
    fn create_window(&self, marker: MainMarker) -> MainActor<Window>;
}

pub struct Window {
    backend: Box<dyn WindowBackend>,
}

impl Window {
    pub(crate) fn new(backend: Box<dyn WindowBackend>) -> Self {
        Window { backend }
    }

    pub fn show(&self) {
        self.backend.show();
    }
}

pub(crate) trait WindowBackend {
    fn show(&self);
}

pub trait MessageHandler<M> {
    fn on_init(&mut self, _cx: impl Context<M>) {}
    fn handle_event(&self, _cx: impl Context<M>) {}
    fn will_terminate(&mut self, _cx: impl Context<M>) {}
}
