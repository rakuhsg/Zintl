#[cfg(target_os = "macos")]
use core_foundation_sys::runloop::CFRunLoopGetMain;
#[cfg(target_os = "macos")]
use messageloop_appkit::{Context, MessageLoopAppkit, MessageLoopHandler, SendError, Sender};
#[cfg(target_os = "macos")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "macos")]
use std::thread;

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
    // the real CFRunLoopSource callback rather than a Rust test worker thread.
    let values = Arc::new(Mutex::new(Vec::new()));
    // SAFETY: Core Foundation returns a valid borrowed process main run loop.
    let run_loop = unsafe { CFRunLoopGetMain() };
    // SAFETY: `run_loop` is the valid borrowed main run loop required by `new`.
    let message_loop = unsafe {
        MessageLoopAppkit::new(
            run_loop,
            Handler {
                main_thread: thread::current().id(),
                values: values.clone(),
            },
        )
    };
    let sender = message_loop.sender();
    let worker_sender = sender.clone();
    let worker = thread::spawn(move || {
        worker_sender.send(Message::Value(1)).unwrap();
        worker_sender.send(Message::SelfSend).unwrap();
        worker_sender.send(Message::Value(2)).unwrap();
    });

    message_loop.run();
    worker.join().unwrap();
    assert_eq!(*values.lock().unwrap(), vec![1, 2, 99]);
    assert_eq!(sender.send(Message::Value(3)), Err(SendError::Closed));
}

#[cfg(not(target_os = "macos"))]
fn main() {}
