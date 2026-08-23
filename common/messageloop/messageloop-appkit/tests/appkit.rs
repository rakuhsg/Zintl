#[cfg(target_os = "macos")]
use messageloop_appkit::{
    Context, MessageLoopAppkit, MessageLoopError, MessageLoopHandler, SendError, Sender,
};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use zpd_appkit::actor::ActorError;
#[cfg(target_os = "macos")]
use zpd_appkit::runloop::Application;

#[cfg(target_os = "macos")]
enum Message {
    Value(usize),
    SelfSend,
    Stop,
}

#[cfg(target_os = "macos")]
struct Handler {
    main_thread: thread::ThreadId,
    values: Arc<Mutex<Vec<usize>>>,
}

#[cfg(target_os = "macos")]
struct Noop;

#[cfg(target_os = "macos")]
impl MessageLoopHandler<()> for Noop {}

#[cfg(target_os = "macos")]
impl MessageLoopHandler<Message> for Handler {
    fn init(&mut self, _cx: &Context<'_, Message>) {
        assert_eq!(thread::current().id(), self.main_thread);
    }

    fn on(&mut self, cx: &Context<'_, Message>, message: Message) {
        assert_eq!(thread::current().id(), self.main_thread);
        match message {
            Message::Value(value) => self.values.lock().unwrap().push(value),
            Message::SelfSend => {
                let sender = cx.sender();
                sender.send(Message::Value(99)).unwrap();
                sender.send(Message::Stop).unwrap();
            }
            Message::Stop => cx.request_termination(),
        }
    }

    fn terminate(&mut self, _cx: &Context<'_, Message>) {
        assert_eq!(thread::current().id(), self.main_thread);
    }
}

#[cfg(target_os = "macos")]
fn main() {
    // Verifies FIFO delivery, self-send, closure, and main-thread affinity via
    // zpd-appkit's safe run-loop source rather than a Rust test worker.
    let values = Arc::new(Mutex::new(Vec::new()));
    let application = Application::new(()).unwrap();
    assert!(application.run_loop().is_current());
    let message_loop = MessageLoopAppkit::new(
        &application,
        Handler {
            main_thread: thread::current().id(),
            values: values.clone(),
        },
    )
    .unwrap();
    let sender = message_loop.sender();
    let worker_sender = sender.clone();
    let worker = thread::spawn(move || {
        worker_sender.send(Message::Value(1)).unwrap();
        worker_sender.send(Message::SelfSend).unwrap();
        worker_sender.send(Message::Value(2)).unwrap();
    });

    message_loop.run().unwrap();
    worker.join().unwrap();
    assert_eq!(*values.lock().unwrap(), vec![1, 2, 99]);
    assert_eq!(sender.send(Message::Value(3)), Err(SendError::Closed));

    // Verifies a message loop cannot use an NSApp ActorRef from a dropped session.
    drop(application);
    let application = Application::new(()).unwrap();
    let stale_loop = MessageLoopAppkit::new(&application, Noop).unwrap();
    drop(application);
    assert_eq!(
        stale_loop.run(),
        Err(MessageLoopError::Application(ActorError::NotActive))
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {}
