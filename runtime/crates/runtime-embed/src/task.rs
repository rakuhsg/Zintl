use crate::RuntimeError;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::task::{Context, Poll, Waker};
use std::thread::{self, JoinHandle};
use std::time::Duration;

struct TaskState<T> {
    result: Option<Result<T, RuntimeError>>,
    settled: bool,
    waker: Option<Waker>,
}

struct Shared<T> {
    state: Mutex<TaskState<T>>,
    completed: Condvar,
    cancelled: Arc<AtomicBool>,
    timer: Mutex<Option<JoinHandle<()>>>,
}

/// Awaitable result produced by a bounded runtime executor.
pub struct RuntimeTask<T> {
    shared: Arc<Shared<T>>,
}

impl<T> RuntimeTask<T> {
    pub(crate) fn pair() -> (Self, TaskCompletion<T>, Cancellation) {
        let cancelled = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Shared {
            state: Mutex::new(TaskState {
                result: None,
                settled: false,
                waker: None,
            }),
            completed: Condvar::new(),
            cancelled: cancelled.clone(),
            timer: Mutex::new(None),
        });
        (
            Self {
                shared: shared.clone(),
            },
            TaskCompletion { shared },
            Cancellation(cancelled),
        )
    }

    /// Blocks the calling host thread until the task settles.
    ///
    /// Engine/UI threads should `.await` the task instead.
    ///
    /// # Errors
    ///
    /// Returns the sanitized runtime error produced by the operation.
    pub fn wait(self) -> Result<T, RuntimeError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        while state.result.is_none() {
            state = self
                .shared
                .completed
                .wait(state)
                .map_err(|_| RuntimeError::Internal)?;
        }
        let result = state.result.take().ok_or(RuntimeError::Internal)?;
        drop(state);
        self.join_timer()?;
        result
    }

    /// Takes a terminal result without blocking, or returns `None` while pending.
    ///
    /// # Errors
    ///
    /// Reports poisoned task state or timer teardown failure.
    pub fn try_take(&mut self) -> Result<Option<Result<T, RuntimeError>>, RuntimeError> {
        let result = self
            .shared
            .state
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .result
            .take();
        if result.is_some() {
            self.join_timer()?;
        }
        Ok(result)
    }

    /// Cooperatively requests cancellation. Exactly one terminal result is retained.
    pub fn cancel(&self) {
        self.shared.cancelled.store(true, Ordering::Release);
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        if !state.settled {
            state.settled = true;
            state.result = Some(Err(RuntimeError::Cancelled));
            let waker = state.waker.take();
            drop(state);
            self.shared.completed.notify_all();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }

    fn join_timer(&self) -> Result<(), RuntimeError> {
        let timer = self
            .shared
            .timer
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .take();
        if let Some(timer) = timer {
            if timer.thread().id() == thread::current().id() {
                return Ok(());
            }
            timer.join().map_err(|_| RuntimeError::Internal)?;
        }
        Ok(())
    }
}

impl<T> Future for RuntimeTask<T> {
    type Output = Result<T, RuntimeError>;

    fn poll(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Self::Output> {
        let Ok(mut state) = self.shared.state.lock() else {
            return Poll::Ready(Err(RuntimeError::Internal));
        };
        if let Some(result) = state.result.take() {
            drop(state);
            Poll::Ready(self.join_timer().and(result))
        } else {
            state.waker = Some(context.waker().clone());
            Poll::Pending
        }
    }
}

impl<T> Drop for RuntimeTask<T> {
    fn drop(&mut self) {
        self.cancel();
        let _ = self.join_timer();
    }
}

pub(crate) struct TaskCompletion<T> {
    shared: Arc<Shared<T>>,
}

impl<T> Clone for TaskCompletion<T> {
    fn clone(&self) -> Self {
        Self {
            shared: self.shared.clone(),
        }
    }
}

impl<T> TaskCompletion<T> {
    pub(crate) fn install_timer(&self, timer: JoinHandle<()>) -> Result<(), RuntimeError> {
        let mut installed = self
            .shared
            .timer
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        if installed.is_some() {
            return Err(RuntimeError::Internal);
        }
        *installed = Some(timer);
        Ok(())
    }

    pub(crate) fn complete(self, result: Result<T, RuntimeError>) {
        let Ok(mut state) = self.shared.state.lock() else {
            return;
        };
        if state.settled {
            return;
        }
        state.settled = true;
        state.result = Some(result);
        let waker = state.waker.take();
        drop(state);
        self.shared.completed.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    pub(crate) fn timeout(self, duration: Duration, cancellation: &Cancellation) {
        let Ok(state) = self.shared.state.lock() else {
            return;
        };
        let Ok((mut state, _)) =
            self.shared
                .completed
                .wait_timeout_while(state, duration, |state| !state.settled)
        else {
            return;
        };
        if !state.settled {
            cancellation.0.store(true, Ordering::Release);
            state.settled = true;
            state.result = Some(Err(RuntimeError::TimedOut));
            let waker = state.waker.take();
            drop(state);
            self.shared.completed.notify_all();
            if let Some(waker) = waker {
                waker.wake();
            }
        }
    }
}

#[derive(Clone)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Instant;

    // Verifies consuming a completed result cannot make its timeout watcher wait again.
    #[test]
    fn settled_state_survives_result_consumption() {
        let (task, completion, cancellation) = RuntimeTask::pair();
        let timer_completion = completion.clone();
        let timer_installer = completion.clone();
        completion.complete(Ok(7_u8));
        let timer = thread::spawn(move || {
            timer_completion.timeout(Duration::from_secs(5), &cancellation);
        });
        timer_installer.install_timer(timer).expect("timer");
        let started = Instant::now();
        assert_eq!(task.wait().expect("result"), 7);
        assert!(started.elapsed() < Duration::from_secs(1));
    }
}
