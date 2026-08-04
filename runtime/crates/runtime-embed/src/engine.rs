use crate::{RuntimeError, RuntimeHandle, RuntimeTask};
use runtime_engine::{
    DriveBudget, EngineConfiguration, EngineError, EngineEvent, EngineNotifier, EvaluationId,
    EvaluationOutcome, FilesystemRequest, HostCompletion, HostErrorCode, HostRequest,
    HostRequestId, JavaScriptEngineBackend,
};
use runtime_event_loop::timer::TimerQueue;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

const MAXIMUM_PENDING_HOST_REQUESTS: usize = 256;
const MAXIMUM_SETTLED_EVALUATIONS: usize = 256;
const MAXIMUM_CONSOLE_EVENTS: usize = 256;

enum PendingHost {
    Bytes(RuntimeTask<Vec<u8>>),
}

impl PendingHost {
    fn cancel(&self) {
        let Self::Bytes(task) = self;
        task.cancel();
    }
}

/// Work completed by one bounded [`EngineSession::drive`] turn.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DriveReport {
    /// Engine events consumed in this turn.
    pub events: usize,
    /// Event payload bytes charged in this turn.
    pub bytes: usize,
    /// Host completions delivered to the backend.
    pub host_completions: usize,
    /// Whether more immediately drainable work remains.
    pub has_more: bool,
}

/// Coordinates one JavaScript backend with the safe Rust runtime surface.
///
/// This type never exposes engine values, resource-table handles, file
/// descriptors, or platform registrations. [`Self::drive`] is non-blocking;
/// embedders decide where and when to schedule bounded turns.
pub struct EngineSession {
    backend: Box<dyn JavaScriptEngineBackend>,
    runtime: RuntimeHandle,
    pending: HashMap<HostRequestId, PendingHost>,
    evaluations: VecDeque<(EvaluationId, EvaluationOutcome)>,
    console: VecDeque<Vec<u8>>,
    deferred: Option<EngineEvent>,
    timers: TimerQueue,
    timer_origin: Instant,
    running: bool,
}

impl EngineSession {
    /// Starts a backend attached only to the runtime's weak, safe callback surface.
    ///
    /// # Errors
    ///
    /// Rejects a non-running runtime, invalid engine limits, or backend startup failure.
    pub fn attach(
        runtime: RuntimeHandle,
        mut backend: Box<dyn JavaScriptEngineBackend>,
        configuration: EngineConfiguration,
        notifier: Arc<dyn EngineNotifier>,
    ) -> Result<Self, EngineError> {
        validate_configuration(configuration)?;
        backend.start(configuration, notifier)?;
        Ok(Self {
            backend,
            runtime,
            pending: HashMap::new(),
            evaluations: VecDeque::new(),
            console: VecDeque::new(),
            deferred: None,
            timers: TimerQueue::new(configuration.maximum_pending_host_requests)
                .map_err(|_| EngineError::InvalidConfiguration)?,
            timer_origin: Instant::now(),
            running: true,
        })
    }

    /// Provides mutable backend access for evaluation submission and cancellation.
    pub fn backend_mut(&mut self) -> &mut dyn JavaScriptEngineBackend {
        self.backend.as_mut()
    }

    /// Drains completed Rust work and engine events within mandatory item/byte limits.
    ///
    /// # Errors
    ///
    /// Returns a stable backend error or rejects one event larger than the whole budget.
    pub fn drive(&mut self, budget: DriveBudget) -> Result<DriveReport, EngineError> {
        if !self.running {
            return Err(EngineError::InvalidState);
        }
        let mut report = DriveReport::default();
        report.host_completions += self.drain_pending()?;
        report.host_completions += self.fire_due_timers(budget.maximum_events)?;
        while report.events < budget.maximum_events {
            let event = if let Some(event) = self.deferred.take() {
                Some(event)
            } else {
                self.backend.next_event()?
            };
            let Some(event) = event else {
                break;
            };
            let bytes = event.payload_len();
            if bytes > budget.maximum_bytes {
                return Err(EngineError::QuotaExceeded);
            }
            if report.bytes.saturating_add(bytes) > budget.maximum_bytes {
                self.deferred = Some(event);
                report.has_more = true;
                break;
            }
            report.events += 1;
            report.bytes += bytes;
            self.handle_event(event)?;
        }
        report.has_more |= self.deferred.is_some();
        Ok(report)
    }

    /// Removes one settled evaluation for the embedder.
    #[must_use]
    pub fn take_evaluation(&mut self) -> Option<(EvaluationId, EvaluationOutcome)> {
        self.evaluations.pop_front()
    }

    /// Removes one bounded console message for the embedder.
    #[must_use]
    pub fn take_console_output(&mut self) -> Option<Vec<u8>> {
        self.console.pop_front()
    }

    /// Cancels pending work, closes resources and releases engine state.
    ///
    /// # Errors
    ///
    /// Returns a sanitized backend teardown failure. Calls are idempotent.
    pub fn shutdown(&mut self) -> Result<(), EngineError> {
        if !self.running {
            return Ok(());
        }
        self.running = false;
        for task in self.pending.values() {
            task.cancel();
        }
        self.pending.clear();
        let _ = self.timers.shutdown();
        self.backend.shutdown()
    }

    fn handle_event(&mut self, event: EngineEvent) -> Result<(), EngineError> {
        match event {
            EngineEvent::EvaluationSettled { id, outcome } => {
                if self.evaluations.len() == MAXIMUM_SETTLED_EVALUATIONS {
                    return Err(EngineError::QuotaExceeded);
                }
                self.evaluations.push_back((id, outcome));
            }
            EngineEvent::ConsoleOutput(bytes) => {
                if self.console.len() == MAXIMUM_CONSOLE_EVENTS {
                    return Err(EngineError::QuotaExceeded);
                }
                self.console.push_back(bytes);
            }
            EngineEvent::HostRequest { id, request } => self.start_host_request(id, request)?,
        }
        Ok(())
    }

    fn start_host_request(
        &mut self,
        id: HostRequestId,
        request: HostRequest,
    ) -> Result<(), EngineError> {
        if id.0 == 0 || self.pending.contains_key(&id) {
            return Err(EngineError::InvalidRequest);
        }
        if self.pending.len() >= MAXIMUM_PENDING_HOST_REQUESTS {
            return self.complete(id, HostCompletion::Failed(HostErrorCode::QuotaExceeded));
        }
        match request {
            HostRequest::Invoke {
                name,
                version,
                input,
            } => {
                let task = match self.runtime.invoke(name, version, input) {
                    Ok(task) => task,
                    Err(error) => return self.complete_runtime_error(id, error),
                };
                self.pending.insert(id, PendingHost::Bytes(task));
            }
            HostRequest::Sleep { nanoseconds } => {
                let now = self.now_tick();
                let deadline = now
                    .checked_add(nanoseconds)
                    .ok_or(EngineError::QuotaExceeded)?;
                self.timers
                    .schedule_at(id.0, deadline)
                    .map_err(|_| EngineError::QuotaExceeded)?;
            }
            HostRequest::Filesystem(request) => self.start_filesystem(id, request)?,
        }
        Ok(())
    }

    fn start_filesystem(
        &mut self,
        id: HostRequestId,
        request: FilesystemRequest,
    ) -> Result<(), EngineError> {
        match request {
            FilesystemRequest::ReadFile { url, maximum_bytes } => {
                let task = match self.runtime.read_file(url, maximum_bytes) {
                    Ok(task) => task,
                    Err(error) => return self.complete_runtime_error(id, error),
                };
                self.pending.insert(id, PendingHost::Bytes(task));
            }
        }
        Ok(())
    }

    fn drain_pending(&mut self) -> Result<usize, EngineError> {
        let ids: Vec<_> = self.pending.keys().copied().collect();
        let mut ready = Vec::new();
        for id in ids {
            let Some(task) = self.pending.get_mut(&id) else {
                continue;
            };
            let PendingHost::Bytes(task) = task;
            let completion = task
                .try_take()
                .map_err(map_engine_error)?
                .map(completion_from_result);
            if let Some(completion) = completion {
                ready.push((id, completion));
            }
        }
        for (id, completion) in &ready {
            self.pending.remove(id);
            self.complete(*id, completion.clone())?;
        }
        Ok(ready.len())
    }

    fn fire_due_timers(&mut self, maximum: usize) -> Result<usize, EngineError> {
        let fired = self
            .timers
            .pop_due(self.now_tick(), maximum.max(1))
            .map_err(|_| EngineError::Backend)?;
        for timer in &fired {
            self.complete(HostRequestId(timer.request_id), HostCompletion::Unit)?;
        }
        Ok(fired.len())
    }

    fn complete(
        &mut self,
        id: HostRequestId,
        completion: HostCompletion,
    ) -> Result<(), EngineError> {
        self.backend.complete_host_request(id, completion)
    }

    fn complete_runtime_error(
        &mut self,
        id: HostRequestId,
        error: RuntimeError,
    ) -> Result<(), EngineError> {
        self.complete(id, HostCompletion::Failed(map_host_error(error)))
    }

    fn now_tick(&self) -> u64 {
        u64::try_from(self.timer_origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

impl Drop for EngineSession {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn validate_configuration(configuration: EngineConfiguration) -> Result<(), EngineError> {
    if configuration.maximum_pending_evaluations == 0
        || configuration.maximum_pending_host_requests == 0
        || configuration.maximum_source_bytes == 0
        || configuration.maximum_event_bytes == 0
    {
        return Err(EngineError::InvalidConfiguration);
    }
    Ok(())
}

fn completion_from_result(result: Result<Vec<u8>, RuntimeError>) -> HostCompletion {
    match result {
        Ok(bytes) => HostCompletion::Bytes(bytes),
        Err(error) => HostCompletion::Failed(map_host_error(error)),
    }
}

fn map_host_error(error: RuntimeError) -> HostErrorCode {
    match error {
        RuntimeError::PermissionDenied => HostErrorCode::PermissionDenied,
        RuntimeError::QuotaExceeded
        | RuntimeError::InputTooLarge
        | RuntimeError::OutputTooLarge => HostErrorCode::QuotaExceeded,
        RuntimeError::Cancelled => HostErrorCode::Cancelled,
        RuntimeError::TimedOut => HostErrorCode::TimedOut,
        RuntimeError::ShuttingDown => HostErrorCode::ShuttingDown,
        RuntimeError::InvalidArgument
        | RuntimeError::InvalidResource
        | RuntimeError::ResourceClosed
        | RuntimeError::UnknownOperation => HostErrorCode::InvalidRequest,
        _ => HostErrorCode::OperationFailed,
    }
}

fn map_engine_error(error: RuntimeError) -> EngineError {
    match error {
        RuntimeError::QuotaExceeded
        | RuntimeError::InputTooLarge
        | RuntimeError::OutputTooLarge => EngineError::QuotaExceeded,
        RuntimeError::NotSupported => EngineError::Unsupported,
        RuntimeError::InvalidArgument
        | RuntimeError::InvalidResource
        | RuntimeError::UnknownOperation => EngineError::InvalidRequest,
        RuntimeError::InvalidConfiguration => EngineError::InvalidConfiguration,
        RuntimeError::InvalidState | RuntimeError::ShuttingDown => EngineError::InvalidState,
        _ => EngineError::Backend,
    }
}
