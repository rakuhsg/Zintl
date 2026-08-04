//! Bounded request/completion state machine for the embedding ABI.

use crate::codec::{CodecError, CompletionEnvelope, encode_completion};
use crate::{ErrorCode, RuntimeState};
use runtime_event_loop::timer::{TimerError, TimerHandle, TimerQueue};
use std::collections::{HashMap, VecDeque};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeConfig {
    pub max_inflight_requests: u32,
    pub max_completion_bytes: u32,
}

impl RuntimeConfig {
    /// Validates non-zero bounds and their total allocation arithmetic.
    ///
    /// # Errors
    ///
    /// Rejects zero limits or multiplication overflow.
    pub fn validate(self) -> Result<Self, RuntimeError> {
        if self.max_inflight_requests == 0 || self.max_completion_bytes == 0 {
            return Err(RuntimeError::InvalidConfig);
        }
        let _ = usize::try_from(self.max_inflight_requests)
            .ok()
            .and_then(|count| {
                usize::try_from(self.max_completion_bytes)
                    .ok()
                    .and_then(|bytes| bytes.checked_add(28))
                    .and_then(|bytes| count.checked_mul(bytes))
            })
            .ok_or(RuntimeError::InvalidConfig)?;
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PendingRequest {
    op_id: u32,
    timer: Option<TimerHandle>,
}

struct QueuedCompletion {
    request_id: u64,
    encoded: Vec<u8>,
}

/// Runtime state contains no engine value, OS handle, or backend-specific type.
pub struct Runtime {
    config: RuntimeConfig,
    state: RuntimeState,
    pending: HashMap<u64, PendingRequest>,
    completions: VecDeque<QueuedCompletion>,
    completion_bytes: usize,
    timers: TimerQueue,
}

impl Runtime {
    /// Creates a configured runtime without starting a thread or loop.
    ///
    /// # Errors
    ///
    /// Rejects invalid or unbounded configuration.
    pub fn new(config: RuntimeConfig) -> Result<Self, RuntimeError> {
        Ok(Self {
            config: config.validate()?,
            state: RuntimeState::Configured,
            pending: HashMap::new(),
            completions: VecDeque::new(),
            completion_bytes: 0,
            timers: TimerQueue::new(config.max_inflight_requests as usize)
                .map_err(RuntimeError::Timer)?,
        })
    }

    /// Transitions a configured runtime to running without blocking.
    ///
    /// # Errors
    ///
    /// Rejects duplicate start or start after shutdown.
    pub fn start(&mut self) -> Result<(), RuntimeError> {
        if self.state != RuntimeState::Configured {
            return Err(RuntimeError::InvalidState);
        }
        self.state = RuntimeState::Running;
        Ok(())
    }

    /// Records a validated request ID and op identity.
    ///
    /// # Errors
    ///
    /// Rejects non-running state, zero IDs, duplicates, oversize payloads, and
    /// exhaustion including completions not yet drained by the embedder.
    pub fn submit(
        &mut self,
        request_id: u64,
        op_id: u32,
        payload: &[u8],
    ) -> Result<(), RuntimeError> {
        if self.state != RuntimeState::Running {
            return Err(RuntimeError::RuntimeShuttingDown);
        }
        if request_id == 0 || op_id == 0 {
            return Err(RuntimeError::InvalidArgument);
        }
        if payload.len() > self.config.max_completion_bytes as usize {
            return Err(RuntimeError::PayloadTooLarge);
        }
        if self.pending.contains_key(&request_id)
            || self
                .completions
                .iter()
                .any(|completion| completion.request_id == request_id)
        {
            return Err(RuntimeError::DuplicateRequest);
        }
        if self.pending.len() + self.completions.len() >= self.config.max_inflight_requests as usize
        {
            return Err(RuntimeError::QuotaExceeded);
        }
        self.pending
            .insert(request_id, PendingRequest { op_id, timer: None });
        Ok(())
    }

    /// Submits a one-shot timer using an absolute monotonic tick.
    ///
    /// # Errors
    ///
    /// Applies normal submission limits and rolls back transactionally if the
    /// bounded timer queue cannot accept the request.
    pub fn submit_timer(
        &mut self,
        request_id: u64,
        op_id: u32,
        deadline_tick: u64,
    ) -> Result<(), RuntimeError> {
        self.submit(request_id, op_id, &[])?;
        match self.timers.schedule_at(request_id, deadline_tick) {
            Ok(timer) => {
                let pending = self
                    .pending
                    .get_mut(&request_id)
                    .ok_or(RuntimeError::UnknownRequest)?;
                pending.timer = Some(timer);
                Ok(())
            }
            Err(error) => {
                self.pending.remove(&request_id);
                Err(RuntimeError::Timer(error))
            }
        }
    }

    /// Moves at most `maximum` elapsed timers to the completion queue.
    ///
    /// # Errors
    ///
    /// Rejects a zero budget or an internal timer/request invariant failure.
    pub fn fire_due_timers(
        &mut self,
        now_tick: u64,
        maximum: usize,
    ) -> Result<usize, RuntimeError> {
        let fired = self.timers.pop_due(now_tick, maximum)?;
        let count = fired.len();
        for event in fired {
            let pending = self
                .pending
                .get(&event.request_id)
                .ok_or(RuntimeError::UnknownRequest)?;
            if pending.timer.is_none() {
                return Err(RuntimeError::InvalidState);
            }
            self.enqueue_completion(event.request_id, 0, &[])?;
            self.pending.remove(&event.request_id);
        }
        Ok(count)
    }

    /// Returns the next absolute monotonic timer deadline.
    #[must_use]
    pub fn next_timer_deadline(&self) -> Option<u64> {
        self.timers.next_deadline()
    }

    /// Completes a pending host operation exactly once.
    ///
    /// # Errors
    ///
    /// Rejects unknown/late completion and oversized output.
    pub fn complete_host_op(
        &mut self,
        request_id: u64,
        status: u32,
        payload: &[u8],
    ) -> Result<(), RuntimeError> {
        if status > ErrorCode::Internal as u32 {
            return Err(RuntimeError::InvalidArgument);
        }
        if payload.len() > self.config.max_completion_bytes as usize {
            return Err(RuntimeError::PayloadTooLarge);
        }
        let pending = self
            .pending
            .get(&request_id)
            .ok_or(RuntimeError::UnknownRequest)?;
        if pending.timer.is_some() {
            return Err(RuntimeError::InvalidState);
        }
        let _ = pending.op_id;
        self.enqueue_completion(request_id, status, payload)?;
        self.pending.remove(&request_id);
        Ok(())
    }

    /// Cancels a pending operation and queues one stable cancellation result.
    ///
    /// # Errors
    ///
    /// Rejects unknown requests; a later operation result is also rejected.
    pub fn cancel(&mut self, request_id: u64) -> Result<(), RuntimeError> {
        let timer = self
            .pending
            .get(&request_id)
            .ok_or(RuntimeError::UnknownRequest)?
            .timer;
        self.enqueue_completion(request_id, ErrorCode::Cancelled as u32, &[])?;
        if let Some(timer) = timer {
            self.timers.cancel(timer)?;
        }
        self.pending.remove(&request_id);
        Ok(())
    }

    /// Starts shutdown, rejects new work, and settles every pending request.
    ///
    /// # Errors
    ///
    /// Returns an encoding error only if an internal invariant is violated.
    pub fn shutdown(&mut self) -> Result<(), RuntimeError> {
        match self.state {
            RuntimeState::Terminated => return Ok(()),
            RuntimeState::ShuttingDown => {}
            RuntimeState::Configured | RuntimeState::Running => {
                self.state = RuntimeState::ShuttingDown;
            }
        }
        let _ = self.timers.shutdown();
        let mut request_ids: Vec<u64> = self.pending.keys().copied().collect();
        request_ids.sort_unstable();
        for request_id in request_ids {
            self.enqueue_completion(request_id, ErrorCode::RuntimeShuttingDown as u32, &[])?;
            self.pending.remove(&request_id);
        }
        if self.completions.is_empty() {
            self.state = RuntimeState::Terminated;
        }
        Ok(())
    }

    /// Returns the next encoded completion without waiting.
    #[must_use]
    pub fn next_completion(&mut self) -> Option<Vec<u8>> {
        let completion = self.completions.pop_front()?;
        self.completion_bytes -= completion.encoded.len();
        if self.state == RuntimeState::ShuttingDown && self.completions.is_empty() {
            self.state = RuntimeState::Terminated;
        }
        Some(completion.encoded)
    }

    /// Returns the required buffer size for the next completion without consuming it.
    #[must_use]
    pub fn next_completion_len(&self) -> Option<usize> {
        self.completions
            .front()
            .map(|completion| completion.encoded.len())
    }

    #[must_use]
    pub const fn state(&self) -> RuntimeState {
        self.state
    }

    fn enqueue_completion(
        &mut self,
        request_id: u64,
        status: u32,
        payload: &[u8],
    ) -> Result<(), RuntimeError> {
        let encoded = encode_completion(&CompletionEnvelope {
            request_id,
            status,
            payload: payload.to_vec(),
        })?;
        let maximum = self.config.max_inflight_requests as usize
            * (self.config.max_completion_bytes as usize + 28);
        let next_bytes = self
            .completion_bytes
            .checked_add(encoded.len())
            .ok_or(RuntimeError::QuotaExceeded)?;
        if self.completions.len() >= self.config.max_inflight_requests as usize
            || next_bytes > maximum
        {
            return Err(RuntimeError::QuotaExceeded);
        }
        self.completion_bytes = next_bytes;
        self.completions.push_back(QueuedCompletion {
            request_id,
            encoded,
        });
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    InvalidConfig,
    InvalidState,
    InvalidArgument,
    DuplicateRequest,
    UnknownRequest,
    PayloadTooLarge,
    QuotaExceeded,
    RuntimeShuttingDown,
    Codec(CodecError),
    Timer(TimerError),
}

impl From<CodecError> for RuntimeError {
    fn from(value: CodecError) -> Self {
        Self::Codec(value)
    }
}

impl From<TimerError> for RuntimeError {
    fn from(value: TimerError) -> Self {
        Self::Timer(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{Runtime, RuntimeConfig, RuntimeError};
    use crate::RuntimeState;
    use crate::codec::decode_completion;

    fn runtime(max_inflight: u32) -> Runtime {
        let mut runtime = Runtime::new(RuntimeConfig {
            max_inflight_requests: max_inflight,
            max_completion_bytes: 4,
        })
        .expect("runtime");
        runtime.start().expect("started");
        runtime
    }

    #[test]
    // Verifies duplicate request IDs are rejected before dispatch.
    fn duplicate_request_is_rejected() {
        let mut runtime = runtime(2);
        runtime.submit(1, 1, &[]).expect("submitted");
        assert_eq!(
            runtime.submit(1, 1, &[]),
            Err(RuntimeError::DuplicateRequest)
        );
    }

    #[test]
    // Verifies cancel wins exactly once against a late host completion.
    fn cancel_wins_completion_race() {
        let mut runtime = runtime(1);
        runtime.submit(1, 1, &[]).expect("submitted");
        runtime.cancel(1).expect("cancelled");
        assert_eq!(
            runtime.complete_host_op(1, 0, &[]),
            Err(RuntimeError::UnknownRequest)
        );
        let encoded = runtime.next_completion().expect("completion");
        let completion = decode_completion(&encoded, 4).expect("decoded");
        assert_eq!(completion.request_id, 1);
    }

    #[test]
    // Verifies a duplicate host result cannot enqueue a second completion.
    fn duplicate_completion_is_rejected() {
        let mut runtime = runtime(2);
        runtime.submit(1, 1, &[]).expect("submitted");
        runtime.complete_host_op(1, 0, &[1]).expect("completed");
        assert_eq!(
            runtime.complete_host_op(1, 0, &[2]),
            Err(RuntimeError::UnknownRequest)
        );
        let _ = runtime.next_completion().expect("single completion");
        assert!(runtime.next_completion().is_none());
    }

    #[test]
    // Verifies undrained completions apply backpressure to new submissions.
    fn completion_queue_applies_backpressure() {
        let mut runtime = runtime(1);
        runtime.submit(1, 1, &[]).expect("submitted");
        runtime.complete_host_op(1, 0, &[]).expect("completed");
        assert_eq!(runtime.submit(2, 1, &[]), Err(RuntimeError::QuotaExceeded));
        let _ = runtime.next_completion();
        runtime.submit(2, 1, &[]).expect("submitted after drain");
    }

    #[test]
    // Verifies a request ID remains live until its queued completion is drained.
    fn queued_completion_prevents_request_id_reuse() {
        let mut runtime = runtime(2);
        runtime.submit(1, 1, &[]).expect("submitted");
        runtime.complete_host_op(1, 0, &[]).expect("completed");
        assert_eq!(
            runtime.submit(1, 1, &[]),
            Err(RuntimeError::DuplicateRequest)
        );
        let _ = runtime.next_completion().expect("drained");
        runtime.submit(1, 1, &[]).expect("reused after drain");
    }

    #[test]
    // Verifies unknown host status values fail closed without consuming pending work.
    fn unknown_completion_status_is_rejected() {
        let mut runtime = runtime(1);
        runtime.submit(1, 1, &[]).expect("submitted");
        assert_eq!(
            runtime.complete_host_op(1, u32::MAX, &[]),
            Err(RuntimeError::InvalidArgument)
        );
        runtime.complete_host_op(1, 0, &[]).expect("valid retry");
    }

    #[test]
    // Verifies shutdown rejects submits and settles pending work non-blockingly.
    fn shutdown_settles_pending_and_terminates_after_drain() {
        let mut runtime = runtime(2);
        runtime.submit(1, 1, &[]).expect("submitted");
        runtime.shutdown().expect("shutdown");
        assert_eq!(runtime.state(), RuntimeState::ShuttingDown);
        assert_eq!(
            runtime.submit(2, 1, &[]),
            Err(RuntimeError::RuntimeShuttingDown)
        );
        let _ = runtime.next_completion().expect("shutdown completion");
        assert_eq!(runtime.state(), RuntimeState::Terminated);
    }

    #[test]
    // Verifies shutdown completion order is stable rather than HashMap-dependent.
    fn shutdown_completions_are_request_ordered() {
        let mut runtime = runtime(3);
        runtime.submit(3, 1, &[]).expect("third");
        runtime.submit(1, 1, &[]).expect("first");
        runtime.submit(2, 1, &[]).expect("second");
        runtime.shutdown().expect("shutdown");
        let request_ids = (0..3)
            .map(|_| {
                decode_completion(&runtime.next_completion().expect("completion"), 4)
                    .expect("decoded")
                    .request_id
            })
            .collect::<Vec<_>>();
        assert_eq!(request_ids, vec![1, 2, 3]);
    }

    #[test]
    // Verifies completion polling is always empty rather than blocking.
    fn next_completion_is_non_blocking_when_empty() {
        assert!(runtime(1).next_completion().is_none());
    }

    #[test]
    // Verifies elapsed timers become bounded ordinary completions in deterministic order.
    fn timers_join_the_completion_pipeline() {
        let mut runtime = runtime(3);
        runtime.submit_timer(1, 2, 20).expect("first timer");
        runtime.submit_timer(2, 2, 10).expect("second timer");
        runtime.submit_timer(3, 2, 20).expect("third timer");
        assert_eq!(runtime.next_timer_deadline(), Some(10));
        assert_eq!(runtime.fire_due_timers(20, 2), Ok(2));
        let first = decode_completion(&runtime.next_completion().expect("first"), 4)
            .expect("decoded first");
        let second = decode_completion(&runtime.next_completion().expect("second"), 4)
            .expect("decoded second");
        assert_eq!((first.request_id, second.request_id), (2, 1));
        assert_eq!(runtime.next_timer_deadline(), Some(20));
        assert_eq!(runtime.fire_due_timers(20, 2), Ok(1));
    }

    #[test]
    // Verifies cancellation removes a timer and rejects a late host completion.
    fn timer_cancellation_is_exactly_once() {
        let mut runtime = runtime(1);
        runtime.submit_timer(1, 2, 10).expect("timer");
        runtime.cancel(1).expect("cancelled");
        assert_eq!(runtime.next_timer_deadline(), None);
        assert_eq!(runtime.fire_due_timers(10, 1), Ok(0));
        assert_eq!(
            runtime.complete_host_op(1, 0, &[]),
            Err(RuntimeError::UnknownRequest)
        );
    }
}
