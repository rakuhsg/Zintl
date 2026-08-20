#[cfg(target_os = "windows")]
use messageloop_dispatcherqueue::{
    Context, MessageLoopDispatcherQueue, MessageLoopHandler, Sender,
};
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use zpd_winui3::{Application, Window};

#[cfg(target_os = "windows")]
struct Handler {
    terminated: Arc<Mutex<usize>>,
    window: Option<Window>,
}

#[cfg(target_os = "windows")]
impl MessageLoopHandler<()> for Handler {
    fn init(&mut self, cx: &Context<'_, ()>) {
        let window = cx.application().create_window().unwrap();
        window.set_title("external exit test").unwrap();
        window.resize(320, 200).unwrap();
        window.activate().unwrap();
        self.window = Some(window);
    }

    fn on(&mut self, cx: &Context<'_, ()>, (): ()) {
        cx.application().exit().unwrap();
    }

    fn terminate(&mut self, _cx: &Context<'_, ()>) {
        *self.terminated.lock().unwrap() += 1;
        self.window.take();
    }
}

#[cfg(target_os = "windows")]
fn main() {
    // Verifies Application exit outside request_termination still closes the
    // sender and invokes terminate exactly once on RuntimeState drop.
    let terminated = Arc::new(Mutex::new(0));
    let application = Application::new().unwrap();
    let message_loop = MessageLoopDispatcherQueue::new(
        application,
        Handler {
            terminated: terminated.clone(),
            window: None,
        },
    );
    let sender = message_loop.sender();
    sender.send(()).unwrap();
    message_loop.run().unwrap();
    assert_eq!(*terminated.lock().unwrap(), 1);
    assert!(sender.send(()).is_err());
}

#[cfg(not(target_os = "windows"))]
fn main() {}
