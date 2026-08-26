#[cfg(target_os = "macos")]
use messageloop_appkit::{Context, MessageLoopAppkit, MessageLoopHandler, SendError, Sender};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::thread;
#[cfg(target_os = "macos")]
use zpd_appkit::actor::{EventRouteToken, WindowEvent, WindowEventKind};
#[cfg(target_os = "macos")]
use zpd_appkit::runloop::Application;

#[cfg(target_os = "macos")]
enum Message {
    Value(usize),
    SelfSend,
    Stop,
    Window(WindowEvent),
}

#[cfg(target_os = "macos")]
impl From<WindowEvent> for Message {
    fn from(event: WindowEvent) -> Self {
        Self::Window(event)
    }
}

#[cfg(target_os = "macos")]
struct Handler {
    main_thread: thread::ThreadId,
    values: Arc<Mutex<Vec<usize>>>,
    window_events: Arc<Mutex<Vec<WindowEventKind>>>,
}

#[cfg(target_os = "macos")]
#[cfg(target_os = "macos")]
impl MessageLoopHandler<Message> for Handler {
    fn init(&mut self, cx: &Context<'_, '_, Message>) {
        assert_eq!(thread::current().id(), self.main_thread);
        let window = cx
            .create_window_with_event_route(Some(EventRouteToken::new(7)))
            .unwrap();
        assert!(cx.contains_window(window));
        assert!(cx.remove_window(window));
    }

    fn on(&mut self, cx: &Context<'_, '_, Message>, message: Message) {
        assert_eq!(thread::current().id(), self.main_thread);
        match message {
            Message::Value(value) => self.values.lock().unwrap().push(value),
            Message::SelfSend => {
                let sender = cx.sender();
                sender.send(Message::Value(99)).unwrap();
                sender.send(Message::Stop).unwrap();
            }
            Message::Stop => cx.request_termination(),
            Message::Window(event) => {
                assert_eq!(event.route, Some(EventRouteToken::new(7)));
                if matches!(&event.kind, WindowEventKind::DidClose) {
                    assert!(!cx.contains_window(event.window));
                }
                self.window_events.lock().unwrap().push(event.kind);
            }
        }
    }

    fn terminate(&mut self, _cx: &Context<'_, '_, Message>) {
        assert_eq!(thread::current().id(), self.main_thread);
    }
}

#[cfg(target_os = "macos")]
fn main() {
    // Verifies FIFO delivery, self-send, closure, and main-thread affinity via
    // zpd-appkit's safe run-loop source rather than a Rust test worker.
    let values = Arc::new(Mutex::new(Vec::new()));
    let window_events = Arc::new(Mutex::new(Vec::new()));
    let application = Application::new(()).unwrap();
    assert!(application.run_loop().is_current());
    let message_loop = MessageLoopAppkit::new(
        &application,
        Handler {
            main_thread: thread::current().id(),
            values: values.clone(),
            window_events: window_events.clone(),
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
    assert_eq!(
        *window_events.lock().unwrap(),
        vec![
            WindowEventKind::Created,
            WindowEventKind::WillClose,
            WindowEventKind::DidClose,
        ]
    );
    assert_eq!(sender.send(Message::Value(3)), Err(SendError::Closed));

    // The MessageLoopAppkit lifetime statically prevents dropping Application first.
    drop(application);
}

#[cfg(not(target_os = "macos"))]
fn main() {}
