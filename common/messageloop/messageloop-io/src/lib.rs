//! Readiness-based message loop backed by `mio`.

#![forbid(unsafe_code)]
#![allow(clippy::missing_errors_doc)]

pub use messageloop_core::{SendError, Sender, SenderResult};
#[cfg(unix)]
pub use mio::unix::SourceFd;
pub use mio::{Interest, Token, event::Event, event::Source};
use std::collections::VecDeque;
use std::io;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

const WAKE_TOKEN: Token = Token(usize::MAX);
const MAX_MESSAGES_PER_TURN: usize = 64;
const EVENT_CAPACITY: usize = 128;

struct QueueState<M> {
    messages: VecDeque<M>,
    quit_requested: bool,
}
struct SharedState<M> {
    queue: Mutex<QueueState<M>>,
    waker: Arc<mio::Waker>,
}

impl<M> SharedState<M> {
    fn request_termination(&self) {
        let mut queue = self
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        queue.quit_requested = true;
        drop(queue);
        let _ = self.waker.wake();
    }
}

/// Cross-thread sender that queues a message and wakes `Poll`.
pub struct IoSender<M> {
    shared: Weak<SharedState<M>>,
}

impl<M> Clone for IoSender<M> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<M: Send + 'static> Sender for IoSender<M> {
    type Message = M;
    fn send(&self, message: M) -> SenderResult {
        let shared = self.shared.upgrade().ok_or(SendError::Closed)?;
        {
            let mut queue = shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if queue.quit_requested {
                return Err(SendError::Closed);
            }
            queue.messages.push_back(message);
        }
        shared.waker.wake().map_err(|_| SendError::Closed)
    }
}

/// Loop-local access to submission, termination and readiness registration.
pub struct IoContext<'a, M> {
    registry: &'a mio::Registry,
    shared: &'a Arc<SharedState<M>>,
    next_token: &'a mut usize,
    local_only: PhantomData<Rc<()>>,
}

impl<M: Send + 'static> IoContext<'_, M> {
    #[must_use]
    pub fn sender(&self) -> IoSender<M> {
        IoSender {
            shared: Arc::downgrade(self.shared),
        }
    }
    pub fn request_termination(&self) {
        self.shared.request_termination();
    }
    pub fn register(&mut self, source: &mut impl Source, interest: Interest) -> io::Result<Token> {
        let token = Token(*self.next_token);
        *self.next_token = self
            .next_token
            .checked_add(1)
            .ok_or_else(|| io::Error::other("I/O token space exhausted"))?;
        self.registry.register(source, token, interest)?;
        Ok(token)
    }
    pub fn reregister(
        &self,
        source: &mut impl Source,
        token: Token,
        interest: Interest,
    ) -> io::Result<()> {
        self.registry.reregister(source, token, interest)
    }
    pub fn deregister(&self, source: &mut impl Source) -> io::Result<()> {
        self.registry.deregister(source)
    }
}

/// Serial callbacks owned exclusively by the thread running [`MessageLoopIo`].
pub trait IoMessageHandler<M> {
    fn init(&mut self, _cx: &mut IoContext<'_, M>) -> io::Result<()> {
        Ok(())
    }
    fn on(&mut self, _cx: &mut IoContext<'_, M>, _message: M) -> io::Result<()> {
        Ok(())
    }
    /// Handles readiness, not completion; implementations must tolerate `WouldBlock`.
    fn on_ready(&mut self, _cx: &mut IoContext<'_, M>, _event: &Event) -> io::Result<()> {
        Ok(())
    }
    fn terminate(&mut self, _cx: &mut IoContext<'_, M>) -> io::Result<()> {
        Ok(())
    }
}

/// A bounded-fair message and I/O readiness loop.
pub struct MessageLoopIo<M, H> {
    poll: mio::Poll,
    events: mio::Events,
    shared: Arc<SharedState<M>>,
    handler: H,
    next_token: usize,
}

impl<M: Send + 'static, H: IoMessageHandler<M>> MessageLoopIo<M, H> {
    pub fn new(handler: H) -> io::Result<Self> {
        let poll = mio::Poll::new()?;
        let waker = Arc::new(mio::Waker::new(poll.registry(), WAKE_TOKEN)?);
        Ok(Self {
            poll,
            events: mio::Events::with_capacity(EVENT_CAPACITY),
            shared: Arc::new(SharedState {
                queue: Mutex::new(QueueState {
                    messages: VecDeque::new(),
                    quit_requested: false,
                }),
                waker,
            }),
            handler,
            next_token: 0,
        })
    }
    #[must_use]
    pub fn sender(&self) -> IoSender<M> {
        IoSender {
            shared: Arc::downgrade(&self.shared),
        }
    }
    pub fn run(mut self) -> io::Result<()> {
        self.with_context(IoMessageHandler::init)?;
        loop {
            for _ in 0..MAX_MESSAGES_PER_TURN {
                let message = {
                    let mut queue = self
                        .shared
                        .queue
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if queue.quit_requested {
                        None
                    } else {
                        queue.messages.pop_front()
                    }
                };
                let Some(message) = message else { break };
                self.with_context(|handler, cx| handler.on(cx, message))?;
            }
            if self.quit_requested() {
                break;
            }
            let timeout = self.messages_remain().then_some(Duration::ZERO);
            self.poll.poll(&mut self.events, timeout)?;
            let ready: Vec<_> = self
                .events
                .iter()
                .filter(|event| event.token() != WAKE_TOKEN)
                .cloned()
                .collect();
            for event in &ready {
                self.with_context(|handler, cx| handler.on_ready(cx, event))?;
                if self.quit_requested() {
                    break;
                }
            }
        }
        self.with_context(IoMessageHandler::terminate)?;
        self.shared.request_termination();
        Ok(())
    }
    fn with_context<R>(
        &mut self,
        callback: impl FnOnce(&mut H, &mut IoContext<'_, M>) -> io::Result<R>,
    ) -> io::Result<R> {
        let mut cx = IoContext {
            registry: self.poll.registry(),
            shared: &self.shared,
            next_token: &mut self.next_token,
            local_only: PhantomData,
        };
        callback(&mut self.handler, &mut cx)
    }
    fn quit_requested(&self) -> bool {
        self.shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .quit_requested
    }
    fn messages_remain(&self) -> bool {
        !self
            .shared
            .queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .messages
            .is_empty()
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use mio::unix::SourceFd;
    use std::io::{Read, Write};
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::thread;

    enum Message {
        Flood(usize),
        Stop,
    }
    struct Handler {
        reader: UnixStream,
        ready_token: Option<Token>,
        observed: mpsc::Sender<&'static str>,
    }
    impl IoMessageHandler<Message> for Handler {
        fn init(&mut self, cx: &mut IoContext<'_, Message>) -> io::Result<()> {
            self.reader.set_nonblocking(true)?;
            let fd = self.reader.as_raw_fd();
            self.ready_token = Some(cx.register(&mut SourceFd(&fd), Interest::READABLE)?);
            Ok(())
        }
        fn on(&mut self, cx: &mut IoContext<'_, Message>, message: Message) -> io::Result<()> {
            match message {
                Message::Flood(left) if left > 0 => {
                    cx.sender().send(Message::Flood(left - 1)).unwrap();
                }
                Message::Flood(_) => {}
                Message::Stop => cx.request_termination(),
            }
            Ok(())
        }
        fn on_ready(&mut self, cx: &mut IoContext<'_, Message>, event: &Event) -> io::Result<()> {
            if Some(event.token()) != self.ready_token {
                return Ok(());
            }
            let mut byte = [0];
            let mut read_any = false;
            loop {
                match self.reader.read(&mut byte) {
                    Ok(1) => read_any = true,
                    Ok(_) => break,
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        if read_any {
                            self.observed.send("ready-then-would-block").unwrap();
                            cx.request_termination();
                        }
                        break;
                    }
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }
    }
    #[test]
    fn sender_wakes_poll_and_termination_exits() {
        // Verifies an external sender wakes a blocked Poll and can request shutdown.
        let (reader, _writer) = UnixStream::pair().unwrap();
        let (observed, _rx) = mpsc::channel();
        let message_loop = MessageLoopIo::new(Handler {
            reader,
            ready_token: None,
            observed,
        })
        .unwrap();
        let sender = message_loop.sender();
        let handle = thread::spawn(move || message_loop.run().unwrap());
        sender.send(Message::Stop).unwrap();
        handle.join().unwrap();
        assert_eq!(sender.send(Message::Stop), Err(SendError::Closed));
    }
    #[test]
    fn bounded_message_drain_does_not_starve_readiness() {
        // Verifies continuously replenished messages still yield to I/O polling.
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let (observed, rx) = mpsc::channel();
        let message_loop = MessageLoopIo::new(Handler {
            reader,
            ready_token: None,
            observed,
        })
        .unwrap();
        let sender = message_loop.sender();
        sender.send(Message::Flood(10_000)).unwrap();
        let handle = thread::spawn(move || message_loop.run().unwrap());
        writer.write_all(&[1]).unwrap();
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "ready-then-would-block"
        );
        handle.join().unwrap();
    }
}
