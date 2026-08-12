//! Small synchronous message loop backed by a mutex and condition variable.

#![forbid(unsafe_code)]

pub use messageloop_core::{SendError, Sender, SenderResult};
use std::collections::VecDeque;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, Weak};

struct QueueState<M> {
    messages: VecDeque<M>,
    quit_requested: bool,
}

struct SharedState<M> {
    state: Mutex<QueueState<M>>,
    condvar: Condvar,
}

impl<M> SharedState<M> {
    fn new() -> Self {
        Self {
            state: Mutex::new(QueueState {
                messages: VecDeque::new(),
                quit_requested: false,
            }),
            condvar: Condvar::new(),
        }
    }

    fn request_termination(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.quit_requested = true;
        drop(state);
        self.condvar.notify_one();
    }
}

/// Thread-safe sender for [`MessageLoopSync`].
pub struct SyncSender<M> {
    state: Weak<SharedState<M>>,
}

impl<M> Clone for SyncSender<M> {
    fn clone(&self) -> Self {
        Self {
            state: self.state.clone(),
        }
    }
}

impl<M: Send + 'static> Sender for SyncSender<M> {
    type Message = M;

    fn send(&self, message: M) -> SenderResult {
        let shared = self.state.upgrade().ok_or(SendError::Closed)?;
        {
            let mut state = shared
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.quit_requested {
                return Err(SendError::Closed);
            }
            state.messages.push_back(message);
        }
        shared.condvar.notify_one();
        Ok(())
    }
}

/// Loop-local context. Its marker intentionally makes it neither `Send` nor `Sync`.
pub struct Context<'a, M> {
    shared: &'a Arc<SharedState<M>>,
    local_only: PhantomData<Rc<()>>,
}

impl<M: Send + 'static> Context<'_, M> {
    #[must_use]
    pub fn sender(&self) -> SyncSender<M> {
        SyncSender {
            state: Arc::downgrade(self.shared),
        }
    }

    pub fn request_termination(&self) {
        self.shared.request_termination();
    }
}

/// Receives messages serially on the thread that calls [`MessageLoopSync::run`].
pub trait MessageHandler<M> {
    fn init(&mut self, _cx: &Context<'_, M>) {}

    fn on(&mut self, _cx: &Context<'_, M>, _message: M) {}

    fn terminate(&mut self, _cx: &Context<'_, M>) {}
}

/// A single-threaded dispatcher with thread-safe message submission.
pub struct MessageLoopSync<M, H> {
    shared: Arc<SharedState<M>>,
    handler: H,
}

impl<M: Send + 'static, H: MessageHandler<M>> MessageLoopSync<M, H> {
    #[must_use]
    pub fn new(handler: H) -> Self {
        Self {
            shared: Arc::new(SharedState::new()),
            handler,
        }
    }

    #[must_use]
    pub fn sender(&self) -> SyncSender<M> {
        SyncSender {
            state: Arc::downgrade(&self.shared),
        }
    }

    pub fn run(mut self) {
        let cx = Context {
            shared: &self.shared,
            local_only: PhantomData,
        };
        self.handler.init(&cx);

        loop {
            let message = {
                let mut state = self
                    .shared
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                while state.messages.is_empty() && !state.quit_requested {
                    state = self
                        .shared
                        .condvar
                        .wait(state)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
                if state.quit_requested {
                    None
                } else {
                    state.messages.pop_front()
                }
            };

            let Some(message) = message else {
                break;
            };
            self.handler.on(&cx, message);
        }

        self.handler.terminate(&cx);
        self.shared.request_termination();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::thread::{self, ThreadId};
    use std::time::Duration;

    enum Message {
        Value(usize),
        SelfSend,
        Stop,
    }

    struct RecordingHandler {
        thread_ids: Arc<Mutex<Vec<ThreadId>>>,
        values: mpsc::Sender<Vec<usize>>,
        seen: Vec<usize>,
    }

    impl MessageHandler<Message> for RecordingHandler {
        fn init(&mut self, _cx: &Context<'_, Message>) {
            self.thread_ids.lock().unwrap().push(thread::current().id());
        }

        fn on(&mut self, cx: &Context<'_, Message>, message: Message) {
            self.thread_ids.lock().unwrap().push(thread::current().id());
            match message {
                Message::Value(value) => self.seen.push(value),
                Message::SelfSend => cx.sender().send(Message::Value(99)).unwrap(),
                Message::Stop => cx.request_termination(),
            }
        }

        fn terminate(&mut self, _cx: &Context<'_, Message>) {
            self.thread_ids.lock().unwrap().push(thread::current().id());
            self.values.send(self.seen.clone()).unwrap();
        }
    }

    type SpawnedLoop = (
        SyncSender<Message>,
        thread::JoinHandle<()>,
        Arc<Mutex<Vec<ThreadId>>>,
        mpsc::Receiver<Vec<usize>>,
    );

    fn spawn_loop() -> SpawnedLoop {
        let (values_tx, values_rx) = mpsc::channel();
        let thread_ids = Arc::new(Mutex::new(Vec::new()));
        let message_loop = MessageLoopSync::new(RecordingHandler {
            thread_ids: thread_ids.clone(),
            values: values_tx,
            seen: Vec::new(),
        });
        let sender = message_loop.sender();
        let handle = thread::spawn(move || message_loop.run());
        (sender, handle, thread_ids, values_rx)
    }

    #[test]
    fn callbacks_share_the_run_thread_and_messages_are_fifo() {
        // Verifies callback affinity and FIFO dispatch across an external sender.
        let (sender, handle, thread_ids, values) = spawn_loop();
        sender.send(Message::Value(1)).unwrap();
        sender.send(Message::Value(2)).unwrap();
        sender.send(Message::Stop).unwrap();
        handle.join().unwrap();
        assert_eq!(values.recv().unwrap(), vec![1, 2]);
        let ids = thread_ids.lock().unwrap();
        assert!(!ids.is_empty());
        assert!(ids.iter().all(|id| *id == ids[0]));
    }

    #[test]
    fn handler_can_send_without_holding_the_queue_lock() {
        // Verifies callback self-send does not deadlock on the queue mutex.
        let (sender, handle, _thread_ids, values) = spawn_loop();
        sender.send(Message::SelfSend).unwrap();
        sender.send(Message::Value(1)).unwrap();
        thread::sleep(Duration::from_millis(20));
        sender.send(Message::Stop).unwrap();
        handle.join().unwrap();
        assert_eq!(values.recv().unwrap(), vec![1, 99]);
    }

    #[test]
    fn sleeping_loop_wakes_for_messages_and_termination() {
        // Verifies Condvar sleep is interrupted by both message and quit notifications.
        let (sender, handle, _thread_ids, values) = spawn_loop();
        thread::sleep(Duration::from_millis(20));
        sender.send(Message::Value(7)).unwrap();
        sender.send(Message::Stop).unwrap();
        handle.join().unwrap();
        assert_eq!(values.recv().unwrap(), vec![7]);
    }

    #[test]
    fn sender_is_closed_after_loop_destruction() {
        // Verifies weak senders cannot keep a destroyed message loop alive.
        let (sender, handle, _thread_ids, _values) = spawn_loop();
        sender.send(Message::Stop).unwrap();
        handle.join().unwrap();
        assert_eq!(sender.send(Message::Value(1)), Err(SendError::Closed));
    }
}
