//! Bounded blocking worker pool with non-blocking completion drain.

use crate::CompletionNotifier;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};

type JobFn = Box<dyn FnOnce(CancellationToken) -> Result<Vec<u8>, WorkerError> + Send>;

struct Job {
    request_id: u64,
    cancellation: CancellationToken,
    operation: JobFn,
}

#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkerCompletion {
    pub request_id: u64,
    pub result: Result<Vec<u8>, WorkerError>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerError {
    Cancelled,
    QueueFull,
    ShuttingDown,
    InvalidArgument,
    InvalidResource,
    PermissionDenied,
    ResourceClosed,
    NotSupported,
    QuotaExceeded,
    Io,
    Protocol,
    WorkerFailed,
}

struct State {
    jobs: VecDeque<Job>,
    completions: VecDeque<WorkerCompletion>,
    queue_limit: usize,
    completion_limit: usize,
    shutting_down: bool,
}

struct Shared {
    state: Mutex<State>,
    jobs_available: Condvar,
    completion_space: Condvar,
    max_completion_bytes: usize,
    notifier: Arc<dyn CompletionNotifier>,
}

/// A fixed-size worker pool. It owns no engine values and never invokes JSC.
pub struct WorkerPool {
    shared: Arc<Shared>,
    workers: Vec<JoinHandle<()>>,
}

impl WorkerPool {
    /// Creates fixed worker threads and bounded job/completion queues.
    ///
    /// # Errors
    ///
    /// Rejects zero bounds and reports thread creation failure after safely
    /// stopping any workers already created.
    pub fn new(
        worker_count: usize,
        queue_limit: usize,
        completion_limit: usize,
        max_completion_bytes: usize,
        notifier: Arc<dyn CompletionNotifier>,
    ) -> Result<Self, WorkerPoolError> {
        if worker_count == 0
            || queue_limit == 0
            || completion_limit == 0
            || max_completion_bytes == 0
        {
            return Err(WorkerPoolError::InvalidConfig);
        }
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                jobs: VecDeque::new(),
                completions: VecDeque::new(),
                queue_limit,
                completion_limit,
                shutting_down: false,
            }),
            jobs_available: Condvar::new(),
            completion_space: Condvar::new(),
            max_completion_bytes,
            notifier,
        });
        let mut workers = Vec::with_capacity(worker_count);
        for index in 0..worker_count {
            let worker_shared = shared.clone();
            if let Ok(worker) = thread::Builder::new()
                .name(format!("runtime-fs-worker-{index}"))
                .spawn(move || worker_main(&worker_shared))
            {
                workers.push(worker);
            } else {
                stop_shared(&shared);
                for worker in workers {
                    let _ = worker.join();
                }
                return Err(WorkerPoolError::ThreadCreation);
            }
        }
        Ok(Self { shared, workers })
    }

    /// Enqueues one blocking operation without waiting for queue space.
    ///
    /// # Errors
    ///
    /// Rejects zero/duplicate-independent invalid IDs, a full queue, poisoned
    /// state, or submission after shutdown.
    pub fn submit(
        &self,
        request_id: u64,
        operation: impl FnOnce(CancellationToken) -> Result<Vec<u8>, WorkerError> + Send + 'static,
    ) -> Result<CancellationToken, WorkerPoolError> {
        if request_id == 0 {
            return Err(WorkerPoolError::InvalidRequest);
        }
        let cancellation = CancellationToken(Arc::new(AtomicBool::new(false)));
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| WorkerPoolError::Poisoned)?;
        if state.shutting_down {
            return Err(WorkerPoolError::ShuttingDown);
        }
        if state.jobs.len() >= state.queue_limit {
            return Err(WorkerPoolError::QueueFull);
        }
        state.jobs.push_back(Job {
            request_id,
            cancellation: cancellation.clone(),
            operation: Box::new(operation),
        });
        drop(state);
        self.shared.jobs_available.notify_one();
        Ok(cancellation)
    }

    /// Pops one worker completion without blocking.
    ///
    /// # Errors
    ///
    /// Returns an error only if internal synchronization was poisoned.
    pub fn next_completion(&self) -> Result<Option<WorkerCompletion>, WorkerPoolError> {
        let mut state = self
            .shared
            .state
            .lock()
            .map_err(|_| WorkerPoolError::Poisoned)?;
        let completion = state.completions.pop_front();
        drop(state);
        if completion.is_some() {
            self.shared.completion_space.notify_one();
        }
        Ok(completion)
    }

    /// Stops accepting work, wakes workers, and joins every thread.
    pub fn shutdown(&mut self) {
        stop_shared(&self.shared);
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}

impl Drop for WorkerPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn stop_shared(shared: &Shared) {
    if let Ok(mut state) = shared.state.lock() {
        state.shutting_down = true;
        for job in &state.jobs {
            job.cancellation.cancel();
        }
    }
    shared.jobs_available.notify_all();
    shared.completion_space.notify_all();
}

fn worker_main(shared: &Shared) {
    loop {
        let job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            while state.jobs.is_empty() && !state.shutting_down {
                let Ok(next) = shared.jobs_available.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.shutting_down && state.jobs.is_empty() {
                return;
            }
            state.jobs.pop_front()
        };
        let Some(job) = job else {
            continue;
        };
        let result = if job.cancellation.is_cancelled() {
            Err(WorkerError::Cancelled)
        } else {
            (job.operation)(job.cancellation)
        };
        let result = match result {
            Ok(output) if output.len() > shared.max_completion_bytes => {
                Err(WorkerError::QuotaExceeded)
            }
            other => other,
        };
        let completion = WorkerCompletion {
            request_id: job.request_id,
            result,
        };
        let pushed = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            while state.completions.len() >= state.completion_limit && !state.shutting_down {
                let Ok(next) = shared.completion_space.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.shutting_down {
                false
            } else {
                state.completions.push_back(completion);
                true
            }
        };
        if pushed {
            shared.notifier.notify_drain_needed();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkerPoolError {
    InvalidConfig,
    InvalidRequest,
    QueueFull,
    ShuttingDown,
    Poisoned,
    ThreadCreation,
}

#[cfg(test)]
mod tests {
    use super::{WorkerError, WorkerPool, WorkerPoolError};
    use crate::CompletionNotifier;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    struct ChannelNotifier(mpsc::Sender<()>);

    impl CompletionNotifier for ChannelNotifier {
        fn notify_drain_needed(&self) {
            let _ = self.0.send(());
        }
    }

    #[test]
    // Verifies worker completion notifies and can be drained without blocking.
    fn worker_completion_notifies_drain() {
        let (sender, receiver) = mpsc::channel();
        let mut pool =
            WorkerPool::new(1, 1, 1, 16, Arc::new(ChannelNotifier(sender))).expect("pool");
        pool.submit(1, |_| Ok(vec![1])).expect("submitted");
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("notified");
        let completion = pool.next_completion().expect("drain").expect("completion");
        assert_eq!(completion.request_id, 1);
        assert_eq!(completion.result, Ok(vec![1]));
        pool.shutdown();
    }

    #[test]
    // Verifies blocking jobs execute on a named worker rather than the submitting thread.
    fn jobs_execute_on_dedicated_worker_threads() {
        let (sender, receiver) = mpsc::channel();
        let mut pool =
            WorkerPool::new(1, 1, 1, 64, Arc::new(ChannelNotifier(sender))).expect("pool");
        pool.submit(1, |_| {
            Ok(std::thread::current()
                .name()
                .unwrap_or_default()
                .as_bytes()
                .to_vec())
        })
        .expect("submitted");
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("notified");
        let completion = pool.next_completion().expect("drain").expect("completion");
        assert_eq!(completion.result, Ok(b"runtime-fs-worker-0".to_vec()));
        pool.shutdown();
    }

    #[test]
    // Verifies cancellation is observed by queued work without engine access.
    fn queued_job_observes_cancellation() {
        let (sender, receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let mut pool =
            WorkerPool::new(1, 2, 2, 16, Arc::new(ChannelNotifier(sender))).expect("pool");
        pool.submit(1, move |_| {
            release_receiver
                .recv()
                .map_err(|_| WorkerError::WorkerFailed)?;
            Ok(Vec::new())
        })
        .expect("first");
        let cancellation = pool.submit(2, |_| Ok(Vec::new())).expect("second");
        cancellation.cancel();
        release_sender.send(()).expect("release");
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("first notification");
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("second notification");
        let first = pool.next_completion().expect("drain").expect("first");
        let second = pool.next_completion().expect("drain").expect("second");
        assert_eq!(first.result, Ok(Vec::new()));
        assert_eq!(second.result, Err(WorkerError::Cancelled));
        pool.shutdown();
    }

    #[test]
    // Verifies configured queue capacity rejects excess work without spawning.
    fn full_queue_is_rejected() {
        let (sender, _receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let (started_sender, started_receiver) = mpsc::channel();
        let mut pool =
            WorkerPool::new(1, 1, 1, 16, Arc::new(ChannelNotifier(sender))).expect("pool");
        pool.submit(1, move |_| {
            started_sender
                .send(())
                .map_err(|_| WorkerError::WorkerFailed)?;
            release_receiver
                .recv()
                .map_err(|_| WorkerError::WorkerFailed)?;
            Ok(Vec::new())
        })
        .expect("running");
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("worker started");
        pool.submit(2, |_| Ok(Vec::new())).expect("queued");
        assert_eq!(
            pool.submit(3, |_| Ok(Vec::new())).err(),
            Some(WorkerPoolError::QueueFull)
        );
        release_sender.send(()).expect("release");
        pool.shutdown();
    }

    #[test]
    // Verifies a worker cannot place an oversized allocation in the completion queue.
    fn completion_payload_is_bounded() {
        let (sender, receiver) = mpsc::channel();
        let mut pool =
            WorkerPool::new(1, 1, 1, 1, Arc::new(ChannelNotifier(sender))).expect("pool");
        pool.submit(1, |_| Ok(vec![1, 2])).expect("submitted");
        receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("notified");
        assert_eq!(
            pool.next_completion()
                .expect("drain")
                .expect("completion")
                .result,
            Err(WorkerError::QuotaExceeded)
        );
        pool.shutdown();
    }
}
