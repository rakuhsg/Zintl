//! Engine-neutral JavaScript embedding contracts.
//!
//! An engine backend owns every engine-native context, value, callback and
//! promise root. The Rust runtime exchanges only owned bytes and opaque IDs
//! with a backend. Backends enqueue events and use [`EngineNotifier`] only to
//! wake the embedder; a notifier must never call back into an engine.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::sync::Arc;

/// Canonical versioned wire codec shared by native engine adapters.
pub mod wire;

/// Maximum work an embedder permits in one non-blocking drain turn.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DriveBudget {
    /// Maximum number of engine events processed in one turn.
    pub maximum_events: usize,
    /// Maximum aggregate payload bytes processed in one turn.
    pub maximum_bytes: usize,
}

impl DriveBudget {
    /// Creates a finite drive budget.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::InvalidConfiguration`] when either bound is zero.
    pub const fn new(maximum_events: usize, maximum_bytes: usize) -> Result<Self, EngineError> {
        if maximum_events == 0 || maximum_bytes == 0 {
            return Err(EngineError::InvalidConfiguration);
        }
        Ok(Self {
            maximum_events,
            maximum_bytes,
        })
    }
}

/// Hard limits enforced by an engine backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EngineConfiguration {
    /// Maximum simultaneously pending evaluations.
    pub maximum_pending_evaluations: usize,
    /// Maximum simultaneously pending host requests.
    pub maximum_pending_host_requests: usize,
    /// Maximum UTF-8 source length.
    pub maximum_source_bytes: usize,
    /// Maximum queued event payload bytes.
    pub maximum_event_bytes: usize,
}

impl Default for EngineConfiguration {
    fn default() -> Self {
        Self {
            maximum_pending_evaluations: 64,
            maximum_pending_host_requests: 256,
            maximum_source_bytes: 1024 * 1024,
            maximum_event_bytes: 4 * 1024 * 1024,
        }
    }
}

/// Runtime-local evaluation identity. Zero is invalid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EvaluationId(pub u64);

/// Runtime-local host-request identity. Zero is invalid.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct HostRequestId(pub u64);

/// Runtime-local opaque JavaScript host-object identity. It is not an OS handle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct EngineObjectId(pub u64);

/// Source submitted for evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationRequest {
    /// Unique nonzero request identity.
    pub id: EvaluationId,
    /// Owned UTF-8 JavaScript source.
    pub source: String,
}

/// Stable resource kinds understood by every engine backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineObjectKind {
    /// A directory capability object.
    Directory,
    /// An opened file capability object.
    File,
}

/// Filesystem host operations emitted by JavaScript.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MountRequest {
    /// Reads a file through a host-registered mount URI.
    ReadFile {
        /// Mount URI containing no operating-system path.
        url: String,
        /// Mandatory finite read limit.
        maximum_bytes: usize,
    },
}

/// A validated request emitted by a JavaScript backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostRequest {
    /// Invokes a registered custom operation.
    Invoke {
        /// Registered operation name.
        name: String,
        /// Registered operation version.
        version: u32,
        /// Owned bounded input bytes.
        input: Vec<u8>,
    },
    /// Completes after a bounded monotonic duration.
    Sleep {
        /// Finite monotonic delay in nanoseconds.
        nanoseconds: u64,
    },
    /// Performs an operation through an IO-side mount service.
    Mount(MountRequest),
}

/// Sanitized JavaScript exception information.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JavaScriptException {
    /// Stable exception class, such as `TypeError`.
    pub name: String,
    /// Sanitized message with no native pointer, handle or path disclosure.
    pub message: String,
}

/// Terminal result of one evaluation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationOutcome {
    /// Canonical UTF-8 JSON produced by the backend.
    Value(Vec<u8>),
    /// A sanitized JavaScript exception.
    Exception(JavaScriptException),
    /// Evaluation was cancelled before settlement.
    Cancelled,
}

/// Events drained from an engine backend.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EngineEvent {
    /// An evaluation settled exactly once.
    EvaluationSettled {
        /// Settled evaluation identity.
        id: EvaluationId,
        /// Terminal evaluation result.
        outcome: EvaluationOutcome,
    },
    /// JavaScript requested a trusted host operation.
    HostRequest {
        /// New host request identity.
        id: HostRequestId,
        /// Validated typed request.
        request: HostRequest,
    },
    /// A bounded console message produced by JavaScript.
    ConsoleOutput(Vec<u8>),
}

impl EngineEvent {
    /// Returns bytes charged against a drive budget.
    #[must_use]
    pub fn payload_len(&self) -> usize {
        match self {
            Self::EvaluationSettled { outcome, .. } => match outcome {
                EvaluationOutcome::Value(bytes) => bytes.len(),
                EvaluationOutcome::Exception(error) => error.name.len() + error.message.len(),
                EvaluationOutcome::Cancelled => 0,
            },
            Self::HostRequest { request, .. } => match request {
                HostRequest::Invoke { name, input, .. } => name.len() + input.len(),
                HostRequest::Sleep { .. } => 8,
                HostRequest::Mount(request) => match request {
                    MountRequest::ReadFile { url, .. } => url.len(),
                },
            },
            Self::ConsoleOutput(bytes) => bytes.len(),
        }
    }
}

/// Stable host failure categories exposed to an engine backend.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HostErrorCode {
    /// Permission was absent or denied.
    PermissionDenied,
    /// A request, receiver or state was invalid.
    InvalidRequest,
    /// A bounded queue, table or byte quota was exhausted.
    QuotaExceeded,
    /// Work was cancelled.
    Cancelled,
    /// Work exceeded its deadline.
    TimedOut,
    /// Runtime shutdown has begun.
    ShuttingDown,
    /// The operation failed without exposing backend details.
    OperationFailed,
}

/// Result returned to a pending JavaScript host Promise.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostCompletion {
    /// Operation returned owned bytes.
    Bytes(Vec<u8>),
    /// Operation returned no value.
    Unit,
    /// Operation minted an opaque engine object.
    Object {
        /// Newly allocated runtime-local object identity.
        id: EngineObjectId,
        /// Resource kind used for private engine branding.
        kind: EngineObjectKind,
    },
    /// Operation failed with a stable category.
    Failed(HostErrorCode),
}

/// Thread-safe wake notification used by a backend.
pub trait EngineNotifier: Send + Sync + 'static {
    /// Signals that at least one event may be ready to drain.
    fn notify(&self);
}

/// JavaScript-engine-neutral lifecycle and event contract.
pub trait JavaScriptEngineBackend {
    /// Starts the backend with finite limits and a wake-only notifier.
    ///
    /// # Errors
    ///
    /// Rejects invalid lifecycle/configuration or backend initialization failure.
    fn start(
        &mut self,
        configuration: EngineConfiguration,
        notifier: Arc<dyn EngineNotifier>,
    ) -> Result<(), EngineError>;

    /// Submits owned JavaScript source without waiting for settlement.
    ///
    /// # Errors
    ///
    /// Rejects invalid IDs/source, lifecycle state or bounded queue exhaustion.
    fn submit_evaluation(&mut self, request: EvaluationRequest) -> Result<(), EngineError>;

    /// Returns the next queued event without blocking.
    ///
    /// # Errors
    ///
    /// Reports malformed backend protocol or unavailable backend state.
    fn next_event(&mut self) -> Result<Option<EngineEvent>, EngineError>;

    /// Settles one pending JavaScript host request.
    ///
    /// # Errors
    ///
    /// Rejects unknown/settled IDs, invalid payloads or unavailable backend state.
    fn complete_host_request(
        &mut self,
        request_id: HostRequestId,
        completion: HostCompletion,
    ) -> Result<(), EngineError>;

    /// Runs the engine's Promise jobs and host microtasks to a checkpoint.
    ///
    /// The caller must invoke this on the same thread as all other JavaScript
    /// entry points, before dispatching the next ordinary message.
    ///
    /// # Errors
    /// Reports an unavailable engine state or backend checkpoint failure.
    fn perform_microtask_checkpoint(&mut self) -> Result<(), EngineError>;

    /// Cancels one pending evaluation.
    ///
    /// # Errors
    ///
    /// Rejects unknown/settled IDs or unavailable backend state.
    fn cancel_evaluation(&mut self, evaluation_id: EvaluationId) -> Result<(), EngineError>;

    /// Rejects pending work and releases engine-owned state. Calls are idempotent.
    ///
    /// # Errors
    ///
    /// Reports sanitized backend teardown failure.
    fn shutdown(&mut self) -> Result<(), EngineError>;
}

/// Stable engine-boundary errors.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EngineError {
    /// A public hard limit was zero or inconsistent.
    InvalidConfiguration,
    /// Operation is invalid for the backend lifecycle state.
    InvalidState,
    /// A request identity was zero, duplicate or unknown.
    InvalidRequest,
    /// A bounded queue or byte budget was exhausted.
    QuotaExceeded,
    /// The selected backend is unavailable on this platform.
    Unsupported,
    /// Backend failed without exposing engine-native details.
    Backend,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::{DriveBudget, EngineError, EngineEvent, EvaluationId, EvaluationOutcome};

    #[test]
    // Verifies zero work limits cannot create an effectively unbounded driver contract.
    fn drive_budget_requires_finite_nonzero_bounds() {
        assert_eq!(
            DriveBudget::new(0, 1),
            Err(EngineError::InvalidConfiguration)
        );
        assert_eq!(
            DriveBudget::new(1, 0),
            Err(EngineError::InvalidConfiguration)
        );
    }

    #[test]
    // Verifies event payload accounting charges owned result bytes.
    fn event_payload_accounting_is_deterministic() {
        let event = EngineEvent::EvaluationSettled {
            id: EvaluationId(1),
            outcome: EvaluationOutcome::Value(vec![1, 2, 3]),
        };
        assert_eq!(event.payload_len(), 3);
    }
}
