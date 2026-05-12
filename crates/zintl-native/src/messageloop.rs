use crate::actor::MainMarker;

pub struct MainTask<M> {
    pub(crate) f: Box<dyn Fn(MainMarker) -> ()>,
    pub(crate) send_after: Option<M>,
}

pub trait Context<M> {
    fn perform_main(&self, f: impl Fn(MainMarker) -> () + 'static, send_after: Option<M>);
}

pub trait MessageHandler<M> {
    fn on_init(&mut self, _cx: impl Context<M>) {}
    fn handle_event(&self, _cx: impl Context<M>) {}
    fn will_terminate(&mut self, _cx: impl Context<M>) {}
}
