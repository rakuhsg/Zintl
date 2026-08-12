//! Shared interfaces for in-process message loops.

#![forbid(unsafe_code)]

use std::error::Error;
use std::fmt;

/// Result returned after a message has been queued.
pub type SenderResult = Result<(), SendError>;

/// Sending failed because the receiving loop is terminating or gone.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SendError {
    Closed,
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed => formatter.write_str("message loop is closed"),
        }
    }
}

impl Error for SendError {}

/// An in-process, thread-safe handle that queues messages for a loop.
pub trait Sender: Send + Sync {
    type Message: Send + 'static;

    /// Queues a message without invoking the receiver inline.
    ///
    /// # Errors
    /// Returns `Closed` after termination has begun or the loop is destroyed.
    fn send(&self, message: Self::Message) -> SenderResult;
}
