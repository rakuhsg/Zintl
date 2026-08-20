#[cfg(target_os = "windows")]
use messageloop_dispatcherqueue::{
    Context, MessageLoopDispatcherQueue, MessageLoopHandler, SendError, Sender,
};
#[cfg(target_os = "windows")]
use std::sync::{Arc, Mutex};
#[cfg(target_os = "windows")]
use std::thread;
#[cfg(target_os = "windows")]
use zpd_winui3::{Application, Window};

#[cfg(target_os = "windows")]
enum Message {
    Value(usize),
    SelfSend,
}

#[cfg(target_os = "windows")]
struct Handler {
    main_thread: thread::ThreadId,
    values: Arc<Mutex<Vec<usize>>>,
    terminated: Arc<Mutex<usize>>,
    window: Option<Window>,
}

#[cfg(target_os = "windows")]
impl MessageLoopHandler<Message> for Handler {
    fn init(&mut self, cx: &Context<'_, Message>) {
        assert_eq!(thread::current().id(), self.main_thread);
        let window = cx.application().create_window().unwrap();
        window
            .set_title("messageloop-dispatcherqueue test")
            .unwrap();
        window.resize(320, 200).unwrap();
        window.activate().unwrap();
        self.window = Some(window);
        let sender = cx.sender();
        thread::spawn(move || sender.send(Message::Value(7)).unwrap())
            .join()
            .unwrap();
    }

    fn on(&mut self, cx: &Context<'_, Message>, message: Message) {
        assert_eq!(thread::current().id(), self.main_thread);
        match message {
            Message::Value(value) => {
                self.values.lock().unwrap().push(value);
                if value == 99 {
                    cx.request_termination();
                }
            }
            Message::SelfSend => cx.sender().send(Message::Value(99)).unwrap(),
        }
    }

    fn terminate(&mut self, _cx: &Context<'_, Message>) {
        assert_eq!(thread::current().id(), self.main_thread);
        *self.terminated.lock().unwrap() += 1;
        self.window.take();
    }
}

#[cfg(target_os = "windows")]
fn main() {
    // Verifies pre-launch cross-thread sends, FIFO delivery, self-send,
    // main-thread affinity, termination, and sender closure in one WinUI run.
    let values = Arc::new(Mutex::new(Vec::new()));
    let terminated = Arc::new(Mutex::new(0));
    let application = Application::new().unwrap();
    let message_loop = MessageLoopDispatcherQueue::new(
        application,
        Handler {
            main_thread: thread::current().id(),
            values: values.clone(),
            terminated: terminated.clone(),
            window: None,
        },
    );
    let sender = message_loop.sender();
    let worker_sender = sender.clone();
    thread::spawn(move || {
        worker_sender.send(Message::Value(1)).unwrap();
        worker_sender.send(Message::SelfSend).unwrap();
        worker_sender.send(Message::Value(2)).unwrap();
    })
    .join()
    .unwrap();

    message_loop.run().unwrap();

    assert_eq!(*values.lock().unwrap(), vec![1, 2, 7, 99]);
    assert_eq!(*terminated.lock().unwrap(), 1);
    assert_eq!(sender.send(Message::Value(3)), Err(SendError::Closed));
}

#[cfg(not(target_os = "windows"))]
fn main() {}
