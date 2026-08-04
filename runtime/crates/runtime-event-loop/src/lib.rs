//! Non-blocking embedding callback and completion contracts.

#![forbid(unsafe_code)]

use std::collections::HashMap;

pub mod timer;
pub mod worker;

/// Thread-safe notification only; implementations must not call an engine API.
pub trait CompletionNotifier: Send + Sync + 'static {
    fn notify_drain_needed(&self);
}

/// An executor for embedder-defined work. Implementations must enqueue and
/// return without running expensive work on the caller or JS executor.
pub trait HostOpExecutor: Send + Sync + 'static {
    /// Enqueues owned work and returns without executing heavy work inline.
    ///
    /// # Errors
    ///
    /// Returns an error when the bounded executor cannot accept the job.
    fn enqueue(&self, job: HostOpJob) -> Result<(), CallbackError>;
}

/// Owned, engine-neutral job delivered to a trusted host executor.
pub struct HostOpJob {
    pub request_id: u64,
    pub payload: Vec<u8>,
    task: Option<Box<dyn FnOnce() + Send>>,
}

impl HostOpJob {
    #[must_use]
    pub fn new(request_id: u64, payload: Vec<u8>, task: impl FnOnce() + Send + 'static) -> Self {
        Self {
            request_id,
            payload,
            task: Some(Box::new(task)),
        }
    }

    /// Runs the trusted task at most once on the host executor.
    pub fn run(mut self) {
        if let Some(task) = self.task.take() {
            task();
        }
    }
}

/// Configuration is bounded and has no unlimited sentinel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DrainBudget {
    pub max_completions: u32,
    pub max_bytes: u32,
}

impl DrainBudget {
    #[must_use]
    pub fn new(max_completions: u32, max_bytes: u32) -> Option<Self> {
        (max_completions > 0 && max_bytes > 0).then_some(Self {
            max_completions,
            max_bytes,
        })
    }
}

/// Terminal result produced by a host callback.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostCompletion {
    Success(Vec<u8>),
    Failed(CallbackError),
    Cancelled,
    TimedOut,
    RuntimeShuttingDown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallbackError {
    ExecutorUnavailable,
    HandlerFailed,
    OutputTooLarge,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RequestState {
    Pending,
    Settled(HostCompletion),
}

#[derive(Clone, Debug)]
struct RequestEntry {
    deadline: u64,
    max_output_bytes: usize,
    state: RequestState,
}

/// Result of racing completion, cancel, timeout, or shutdown.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TransitionResult {
    Won,
    Late,
    UnknownRequest,
}

/// Bounded exactly-once state machine shared by custom ops and permission
/// callbacks. Time values are supplied by runtime core's monotonic clock.
#[derive(Debug)]
pub struct CallbackTracker {
    max_inflight: usize,
    requests: HashMap<u64, RequestEntry>,
}

impl CallbackTracker {
    /// Creates an empty callback tracker.
    ///
    /// # Errors
    ///
    /// Rejects a zero in-flight limit.
    pub fn new(max_inflight: usize) -> Result<Self, TrackerError> {
        if max_inflight == 0 {
            return Err(TrackerError::QuotaExceeded);
        }
        Ok(Self {
            max_inflight,
            requests: HashMap::new(),
        })
    }

    /// Registers a request before invoking an external callback.
    ///
    /// # Errors
    ///
    /// Rejects zero IDs, duplicate IDs, elapsed deadlines, zero output limits,
    /// and bounded table exhaustion.
    pub fn register(
        &mut self,
        request_id: u64,
        deadline: u64,
        now: u64,
        max_output_bytes: usize,
    ) -> Result<(), TrackerError> {
        if request_id == 0 || deadline <= now || max_output_bytes == 0 {
            return Err(TrackerError::InvalidRequest);
        }
        if self.requests.contains_key(&request_id) {
            return Err(TrackerError::DuplicateRequest);
        }
        if self.requests.len() >= self.max_inflight {
            return Err(TrackerError::QuotaExceeded);
        }
        self.requests.insert(
            request_id,
            RequestEntry {
                deadline,
                max_output_bytes,
                state: RequestState::Pending,
            },
        );
        Ok(())
    }

    /// Attempts to complete a host callback.
    pub fn complete(&mut self, request_id: u64, completion: HostCompletion) -> TransitionResult {
        let Some(entry) = self.requests.get_mut(&request_id) else {
            return TransitionResult::UnknownRequest;
        };
        if !matches!(entry.state, RequestState::Pending) {
            return TransitionResult::Late;
        }
        let completion = match completion {
            HostCompletion::Success(payload) if payload.len() > entry.max_output_bytes => {
                HostCompletion::Failed(CallbackError::OutputTooLarge)
            }
            other => other,
        };
        entry.state = RequestState::Settled(completion);
        TransitionResult::Won
    }

    /// Attempts to cancel a pending callback.
    pub fn cancel(&mut self, request_id: u64) -> TransitionResult {
        self.complete(request_id, HostCompletion::Cancelled)
    }

    /// Settles every elapsed pending callback as timed out.
    #[must_use]
    pub fn expire(&mut self, now: u64) -> usize {
        let mut expired = 0;
        for entry in self.requests.values_mut() {
            if matches!(entry.state, RequestState::Pending) && now >= entry.deadline {
                entry.state = RequestState::Settled(HostCompletion::TimedOut);
                expired += 1;
            }
        }
        expired
    }

    /// Settles every pending callback during shutdown.
    #[must_use]
    pub fn shutdown(&mut self) -> usize {
        let mut settled = 0;
        for entry in self.requests.values_mut() {
            if matches!(entry.state, RequestState::Pending) {
                entry.state = RequestState::Settled(HostCompletion::RuntimeShuttingDown);
                settled += 1;
            }
        }
        settled
    }

    /// Removes and returns one terminal result for queueing to the JS adapter.
    pub fn take_terminal(&mut self, request_id: u64) -> Option<HostCompletion> {
        let is_settled = self
            .requests
            .get(&request_id)
            .is_some_and(|entry| matches!(entry.state, RequestState::Settled(_)));
        if !is_settled {
            return None;
        }
        let entry = self.requests.remove(&request_id)?;
        let RequestState::Settled(completion) = entry.state else {
            return None;
        };
        Some(completion)
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        self.requests
            .values()
            .filter(|entry| matches!(entry.state, RequestState::Pending))
            .count()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TrackerError {
    InvalidRequest,
    DuplicateRequest,
    QuotaExceeded,
}

#[cfg(test)]
mod tests {
    use super::{CallbackError, CallbackTracker, HostCompletion, TrackerError, TransitionResult};

    #[test]
    // Verifies completion wins once and a duplicate completion is late.
    fn callback_completes_exactly_once() {
        let mut tracker = CallbackTracker::new(1).expect("tracker");
        tracker.register(1, 10, 0, 4).expect("registered");
        assert_eq!(
            tracker.complete(1, HostCompletion::Success(vec![1])),
            TransitionResult::Won
        );
        assert_eq!(tracker.cancel(1), TransitionResult::Late);
        assert_eq!(
            tracker.take_terminal(1),
            Some(HostCompletion::Success(vec![1]))
        );
    }

    #[test]
    // Verifies an oversized host result fails rather than bypassing its limit.
    fn callback_output_is_bounded() {
        let mut tracker = CallbackTracker::new(1).expect("tracker");
        tracker.register(1, 10, 0, 1).expect("registered");
        assert_eq!(
            tracker.complete(1, HostCompletion::Success(vec![1, 2])),
            TransitionResult::Won
        );
        assert_eq!(
            tracker.take_terminal(1),
            Some(HostCompletion::Failed(CallbackError::OutputTooLarge))
        );
    }

    #[test]
    // Verifies timeout wins the race and rejects a later result.
    fn timeout_rejects_late_completion() {
        let mut tracker = CallbackTracker::new(1).expect("tracker");
        tracker.register(1, 10, 0, 1).expect("registered");
        assert_eq!(tracker.expire(10), 1);
        assert_eq!(
            tracker.complete(1, HostCompletion::Success(vec![])),
            TransitionResult::Late
        );
        assert_eq!(tracker.take_terminal(1), Some(HostCompletion::TimedOut));
    }

    #[test]
    // Verifies shutdown settles pending callbacks without double settlement.
    fn shutdown_settles_pending_callback() {
        let mut tracker = CallbackTracker::new(1).expect("tracker");
        tracker.register(1, 10, 0, 1).expect("registered");
        assert_eq!(tracker.shutdown(), 1);
        assert_eq!(tracker.cancel(1), TransitionResult::Late);
        assert_eq!(
            tracker.take_terminal(1),
            Some(HostCompletion::RuntimeShuttingDown)
        );
    }

    #[test]
    // Verifies duplicate IDs and in-flight exhaustion fail closed.
    fn duplicate_and_exhausted_requests_are_rejected() {
        let mut tracker = CallbackTracker::new(1).expect("tracker");
        tracker.register(1, 10, 0, 1).expect("registered");
        assert_eq!(
            tracker.register(1, 10, 0, 1),
            Err(TrackerError::DuplicateRequest)
        );
        assert_eq!(
            tracker.register(2, 10, 0, 1),
            Err(TrackerError::QuotaExceeded)
        );
    }

    #[test]
    // Verifies settled-but-undrained callbacks still consume bounded tracker capacity.
    fn terminal_results_apply_backpressure_until_drained() {
        let mut tracker = CallbackTracker::new(1).expect("tracker");
        tracker.register(1, 10, 0, 1).expect("registered");
        assert_eq!(
            tracker.complete(1, HostCompletion::Success(Vec::new())),
            TransitionResult::Won
        );
        assert_eq!(
            tracker.register(2, 10, 0, 1),
            Err(TrackerError::QuotaExceeded)
        );
        let _ = tracker.take_terminal(1).expect("drained");
        tracker.register(2, 10, 0, 1).expect("accepted after drain");
    }
}
