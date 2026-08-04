//! Safe, engine-neutral Rust embedding API for the Zintl runtime.
//!
//! The embedder configures finite limits, a permission callback and optional
//! custom operations before starting the runtime. JavaScript engines attach
//! through [`EngineSession`], which exchanges typed events and owned bytes; it
//! never exposes an engine value, OS descriptor, native pointer or resource
//! table handle.
//!
//! Permission is deny-by-default. [`PermissionCallback::request_permission`]
//! receives descriptive request data and a one-shot [`PermissionResponder`].
//! An allow response can only attenuate requested rights and quota. The scope
//! remains Rust-owned, and a directory is opened only after approval.
//!
//! # Lifecycle
//!
//! ```no_run
//! use runtime_embed::{
//!     PermissionDecision, PermissionRequest, PermissionResponder, RuntimeBuilder,
//! };
//!
//! let runtime = RuntimeBuilder::new()
//!     .permission_callback(
//!         |request: PermissionRequest, responder: PermissionResponder| {
//!             // A real host should ask its user. This example denies by default.
//!             let decision = if request.operation == "dev.example.read-only" {
//!                 PermissionDecision::Allow {
//!                     rights: request.requested_rights,
//!                     quota: 64 * 1024,
//!                 }
//!             } else {
//!                 PermissionDecision::Deny
//!             };
//!             let _ = responder.respond(decision);
//!         },
//!     )
//!     .build()?;
//! runtime.start()?;
//! // Attach a JavaScriptEngineBackend with EngineSession, then schedule bounded
//! // EngineSession::drive turns from the host's executor.
//! runtime.shutdown()?;
//! # Ok::<(), runtime_embed::RuntimeError>(())
//! ```
//!
//! [`RuntimeTask`] implements `Future`. Its blocking `wait` helper is only for
//! trusted CLI/background threads; UI and engine executors should await tasks
//! or use bounded [`EngineSession::drive`] turns.

#![forbid(unsafe_code)]

mod engine;
mod executor;
mod filesystem;
mod persistence;
mod task;

pub use engine::{DriveReport, EngineSession};
pub use filesystem::{Directory, FileKind, FileMetadata, FileResource};
pub use persistence::{
    DirectoryScopeCodec, ExactPathScopeCodec, PersistenceConfiguration, ScopeCodecError,
};
pub use runtime_engine;
pub use task::RuntimeTask;

use executor::HostExecutor;
use filesystem::FilesystemCompletions;
use runtime_core::RuntimeState;
use runtime_event_loop::worker::WorkerError;
pub use runtime_filesystem::FilesystemRights;
use runtime_filesystem::{ApprovedDirectory, Filesystem, FilesystemError};
use runtime_ops::{
    DispatchContext, DispatchError, HostOpDisposition, OpDescriptor, OpExecution, OpHandler,
    OpRegistry, OpRegistryBuilder, PermissionAuthorizer, RegistrationError, ValidatedOpContext,
};
pub use runtime_permission::{PermissionCodec, PermissionCodecError};
use runtime_resource::{ResourceHandle, ResourceOwner};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak, mpsc};
use std::time::Duration;

const DIRECTORY_PERMISSION: &str = "zintl.permission.fs.directory";
const DIRECTORY_OPERATION: &str = "zintl.builtin.fs.request-directory";
const FIRST_CUSTOM_OP_ID: u32 = 1_024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpLimits {
    pub max_input_bytes: u32,
    pub max_output_bytes: u32,
    pub timeout_nanoseconds: u64,
}

impl OpLimits {
    /// Validates mandatory finite operation limits.
    ///
    /// # Errors
    ///
    /// Rejects zero input/output limits or timeout.
    pub const fn new(
        max_input_bytes: u32,
        max_output_bytes: u32,
        timeout_nanoseconds: u64,
    ) -> Result<Self, RuntimeError> {
        if max_input_bytes == 0
            || max_output_bytes == 0
            || timeout_nanoseconds == 0
            || timeout_nanoseconds > 86_400_000_000_000
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(Self {
            max_input_bytes,
            max_output_bytes,
            timeout_nanoseconds,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RuntimeConfiguration {
    pub maximum_resources: usize,
    pub filesystem_workers: usize,
    pub filesystem_queue_limit: usize,
    pub filesystem_completion_limit: usize,
    pub host_workers: usize,
    pub host_queue_limit: usize,
    pub maximum_operation_bytes: usize,
    pub maximum_audit_events: usize,
    pub permission_timeout_nanoseconds: u64,
}

impl Default for RuntimeConfiguration {
    fn default() -> Self {
        Self {
            maximum_resources: 1_024,
            filesystem_workers: 2,
            filesystem_queue_limit: 128,
            filesystem_completion_limit: 128,
            host_workers: 2,
            host_queue_limit: 128,
            maximum_operation_bytes: 4 * 1_024 * 1_024,
            maximum_audit_events: 1_024,
            permission_timeout_nanoseconds: 30_000_000_000,
        }
    }
}

impl RuntimeConfiguration {
    fn validate(self) -> Result<Self, RuntimeError> {
        if self.maximum_resources == 0
            || self.filesystem_workers == 0
            || self.filesystem_queue_limit == 0
            || self.filesystem_completion_limit == 0
            || self.host_workers == 0
            || self.host_queue_limit == 0
            || self.maximum_operation_bytes == 0
            || self.maximum_audit_events == 0
            || self.maximum_audit_events > 65_536
            || self.permission_timeout_nanoseconds == 0
            || self.permission_timeout_nanoseconds > 86_400_000_000_000
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        Ok(self)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Lifecycle {
    Configured,
    Running,
    ShuttingDown,
    Terminated,
}

impl From<RuntimeState> for Lifecycle {
    fn from(value: RuntimeState) -> Self {
        match value {
            RuntimeState::Configured => Self::Configured,
            RuntimeState::Running => Self::Running,
            RuntimeState::ShuttingDown => Self::ShuttingDown,
            RuntimeState::Terminated => Self::Terminated,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditCategory {
    Lifecycle,
    CustomOperation,
    DirectoryPermission,
    PermissionPersistence,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuditOutcome {
    Started,
    Allowed,
    Denied,
    Succeeded,
    Failed,
    Cancelled,
    Shutdown,
}

/// Redacted audit event; payloads, locators, handles, and secrets are unrepresentable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub sequence: u64,
    pub category: AuditCategory,
    pub outcome: AuditOutcome,
    pub operation: String,
    pub request_id: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRequest {
    pub request_id: u64,
    pub operation: String,
    pub kind: String,
    pub requested_scope: Vec<u8>,
    pub requested_rights: u64,
    pub requested_directory: Option<String>,
    pub reason: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Deny,
    Allow { rights: u64, quota: u64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionResponseError {
    RequestExpired,
}

/// One-shot authority response. It contains no resource or OS handle.
pub struct PermissionResponder {
    sender: Option<mpsc::SyncSender<PermissionDecision>>,
}

impl PermissionResponder {
    /// Completes one permission request. Consuming `self` prevents duplicate replies.
    ///
    /// # Errors
    ///
    /// Returns `RequestExpired` after timeout, cancellation, or shutdown wins.
    pub fn respond(mut self, decision: PermissionDecision) -> Result<(), PermissionResponseError> {
        self.sender
            .take()
            .ok_or(PermissionResponseError::RequestExpired)?
            .try_send(decision)
            .map_err(|_| PermissionResponseError::RequestExpired)
    }
}

pub trait PermissionCallback: Send + Sync + 'static {
    fn request_permission(&self, request: PermissionRequest, responder: PermissionResponder);
}

impl<F> PermissionCallback for F
where
    F: Fn(PermissionRequest, PermissionResponder) + Send + Sync + 'static,
{
    fn request_permission(&self, request: PermissionRequest, responder: PermissionResponder) {
        self(request, responder);
    }
}

#[derive(Clone)]
pub struct HostOpContext {
    pub request_id: u64,
    pub operation: String,
    pub permission: String,
    cancellation: task::Cancellation,
}

impl HostOpContext {
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
}

pub trait HostOp: Send + Sync + 'static {
    /// Executes validated input on the declared host executor.
    ///
    /// # Errors
    ///
    /// Returns a sanitized host/runtime failure; outputs remain quota-checked.
    fn invoke(&self, context: HostOpContext, input: Vec<u8>) -> Result<Vec<u8>, RuntimeError>;
}

impl<F> HostOp for F
where
    F: Fn(HostOpContext, Vec<u8>) -> Result<Vec<u8>, RuntimeError> + Send + Sync + 'static,
{
    fn invoke(&self, context: HostOpContext, input: Vec<u8>) -> Result<Vec<u8>, RuntimeError> {
        self(context, input)
    }
}

struct RegisteredHostOp {
    operation: String,
    permission: String,
    limits: OpLimits,
    handler: Arc<dyn HostOp>,
}

struct PendingHandler;

impl OpHandler for PendingHandler {
    fn start(&self, _context: ValidatedOpContext, _input: Vec<u8>) -> HostOpDisposition {
        HostOpDisposition::Pending
    }
}

pub struct RuntimeBuilder {
    configuration: RuntimeConfiguration,
    permission_callback: Option<Arc<dyn PermissionCallback>>,
    persistence: Option<PersistenceConfiguration>,
    registry: OpRegistryBuilder,
    operations: HashMap<(String, u32), RegisteredHostOp>,
    next_custom_op_id: u32,
}

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self {
            configuration: RuntimeConfiguration::default(),
            permission_callback: None,
            persistence: None,
            registry: OpRegistryBuilder::new(),
            operations: HashMap::new(),
            next_custom_op_id: FIRST_CUSTOM_OP_ID,
        }
    }

    #[must_use]
    pub fn configuration(mut self, configuration: RuntimeConfiguration) -> Self {
        self.configuration = configuration;
        self
    }

    /// Installs the only callback capable of approving new authority.
    #[must_use]
    pub fn permission_callback(mut self, callback: impl PermissionCallback) -> Self {
        self.permission_callback = Some(Arc::new(callback));
        self
    }

    #[must_use]
    pub fn permission_persistence(mut self, configuration: PersistenceConfiguration) -> Self {
        self.persistence = Some(configuration);
        self
    }

    /// Registers a permission-guarded custom byte operation before build.
    ///
    /// # Errors
    ///
    /// Rejects invalid/reserved names, duplicate registration, invalid limits,
    /// or stable-ID exhaustion.
    pub fn register_op(
        mut self,
        name: impl Into<String>,
        version: u32,
        limits: OpLimits,
        permission: impl Into<String>,
        handler: impl HostOp,
    ) -> Result<Self, RuntimeError> {
        let name = name.into();
        let permission = permission.into();
        let stable_id = self.next_custom_op_id;
        self.next_custom_op_id = self
            .next_custom_op_id
            .checked_add(1)
            .ok_or(RuntimeError::InvalidConfiguration)?;
        let descriptor = OpDescriptor::new(
            stable_id,
            &name,
            version,
            1,
            limits.max_input_bytes,
            limits.max_output_bytes,
            &permission,
            OpExecution::HostExecutor,
            limits.timeout_nanoseconds,
        )
        .map_err(RuntimeError::from_registration)?;
        self.registry
            .register_custom(descriptor, PendingHandler)
            .map_err(RuntimeError::from_registration)?;
        self.operations.insert(
            (name.clone(), version),
            RegisteredHostOp {
                operation: name,
                permission,
                limits,
                handler: Arc::new(handler),
            },
        );
        Ok(self)
    }

    /// Builds a configured runtime. Call `start` before requesting work.
    ///
    /// # Errors
    ///
    /// Rejects invalid bounds, random identity failure, or worker construction.
    pub fn build(self) -> Result<EmbeddedRuntime, RuntimeError> {
        let configuration = self.configuration.validate()?;
        let notifier = Arc::new(FilesystemCompletions::default());
        let owner = ResourceOwner::new(random_nonzero_u64()?).ok_or(RuntimeError::Internal)?;
        let filesystem = Filesystem::new(
            owner,
            configuration.maximum_resources,
            configuration.filesystem_workers,
            configuration.filesystem_queue_limit,
            configuration.filesystem_completion_limit,
            configuration.maximum_operation_bytes,
            notifier.clone(),
        )
        .map_err(RuntimeError::from_filesystem)?;
        let host = HostExecutor::new(configuration.host_workers, configuration.host_queue_limit)?;
        let control = Arc::new(Control {
            state: Mutex::new(RuntimeState::Configured),
            audit: Mutex::new(VecDeque::new()),
            next_audit_sequence: AtomicU64::new(1),
            maximum_audit_events: configuration.maximum_audit_events,
        });
        let persistence = self
            .persistence
            .map(persistence::PersistenceState::new)
            .transpose()?;
        let services = Arc::new(Services {
            control: control.clone(),
            filesystem,
            filesystem_completions: notifier,
            host,
            registry: self.registry.freeze(),
            operations: self.operations,
            permission_callback: self.permission_callback,
            persistence,
            next_request: AtomicU64::new(1),
            live_resources: Mutex::new(HashSet::new()),
            shutting_down: AtomicBool::new(false),
            permission_timeout: Duration::from_nanos(configuration.permission_timeout_nanoseconds),
        });
        Ok(EmbeddedRuntime {
            control,
            services: Mutex::new(Some(services)),
        })
    }
}

pub struct EmbeddedRuntime {
    control: Arc<Control>,
    services: Mutex<Option<Arc<Services>>>,
}

impl EmbeddedRuntime {
    /// Starts the runtime without occupying the caller or starting a run loop.
    ///
    /// # Errors
    ///
    /// Rejects duplicate start and start after shutdown.
    pub fn start(&self) -> Result<(), RuntimeError> {
        let mut state = self
            .control
            .state
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        if *state != RuntimeState::Configured {
            return Err(RuntimeError::InvalidState);
        }
        *state = RuntimeState::Running;
        drop(state);
        self.control.record(
            AuditCategory::Lifecycle,
            AuditOutcome::Started,
            "runtime.start",
            None,
        );
        Ok(())
    }

    #[must_use]
    pub fn lifecycle(&self) -> Lifecycle {
        self.control
            .state
            .lock()
            .map_or(Lifecycle::Terminated, |state| (*state).into())
    }

    /// Returns a bounded oldest-to-newest redacted audit snapshot.
    #[must_use]
    pub fn audit_snapshot(&self) -> Vec<AuditEvent> {
        self.control
            .audit
            .lock()
            .map_or_else(|_| Vec::new(), |events| events.iter().cloned().collect())
    }

    /// Returns a weak, cloneable host callback surface for an engine adapter.
    ///
    /// # Errors
    ///
    /// Rejects calls before start or after shutdown begins.
    pub fn handle(&self) -> Result<RuntimeHandle, RuntimeError> {
        let services = self.services()?;
        services.ensure_running()?;
        Ok(RuntimeHandle {
            services: Arc::downgrade(&services),
        })
    }

    /// Requests exact directory authority on the bounded host executor.
    ///
    /// # Errors
    ///
    /// Rejects non-running state or executor saturation before submission.
    pub fn request_directory(
        &self,
        locator: impl Into<PathBuf>,
        rights: FilesystemRights,
    ) -> Result<RuntimeTask<Directory>, RuntimeError> {
        self.handle()?.request_directory(locator, rights)
    }

    /// Invokes a registered custom operation asynchronously.
    ///
    /// # Errors
    ///
    /// Rejects non-running state or executor saturation before submission.
    pub fn invoke(
        &self,
        name: impl Into<String>,
        version: u32,
        input: Vec<u8>,
        requested_scope: Vec<u8>,
        requested_rights: u64,
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        self.handle()?
            .invoke(name, version, input, requested_scope, requested_rights)
    }

    /// Authenticates and exports one live directory permission asynchronously.
    ///
    /// # Errors
    ///
    /// Rejects missing persistence, foreign/closed resources, invalid expiry,
    /// or executor saturation.
    pub fn export_directory_permission(
        &self,
        directory: &Directory,
        expires_at: u64,
        now: u64,
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        self.handle()?
            .export_directory_permission(directory, expires_at, now)
    }

    /// Imports an authenticated permission with rights/quota attenuation.
    ///
    /// # Errors
    ///
    /// Rejects missing persistence, invalid bounds, or executor saturation.
    pub fn import_directory_permission(
        &self,
        blob: Vec<u8>,
        requested_rights: FilesystemRights,
        requested_quota: u64,
        now: u64,
    ) -> Result<RuntimeTask<Directory>, RuntimeError> {
        self.handle()?
            .import_directory_permission(blob, requested_rights, requested_quota, now)
    }

    /// Closes resources, stops bounded host workers, and terminates idempotently.
    ///
    /// # Errors
    ///
    /// Reports internal synchronization or worker termination failure.
    pub fn shutdown(&self) -> Result<(), RuntimeError> {
        {
            let mut state = self
                .control
                .state
                .lock()
                .map_err(|_| RuntimeError::Internal)?;
            if *state == RuntimeState::Terminated {
                return Ok(());
            }
            *state = RuntimeState::ShuttingDown;
        }
        let services = self
            .services
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .take();
        if let Some(services) = services {
            services.shutting_down.store(true, Ordering::Release);
            services.close_all_resources()?;
            services.host.shutdown()?;
        }
        *self
            .control
            .state
            .lock()
            .map_err(|_| RuntimeError::Internal)? = RuntimeState::Terminated;
        self.control.record(
            AuditCategory::Lifecycle,
            AuditOutcome::Shutdown,
            "runtime.shutdown",
            None,
        );
        Ok(())
    }

    fn services(&self) -> Result<Arc<Services>, RuntimeError> {
        self.services
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .as_ref()
            .cloned()
            .ok_or(RuntimeError::ShuttingDown)
    }
}

impl Drop for EmbeddedRuntime {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

#[derive(Clone)]
pub struct RuntimeHandle {
    services: Weak<Services>,
}

impl RuntimeHandle {
    /// Requests exact directory authority through the configured resolver.
    ///
    /// # Errors
    ///
    /// Rejects shutdown state or bounded host-queue saturation.
    pub fn request_directory(
        &self,
        locator: impl Into<PathBuf>,
        rights: FilesystemRights,
    ) -> Result<RuntimeTask<Directory>, RuntimeError> {
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let locator = locator.into();
        let weak = self.services.clone();
        services
            .host
            .submit_with_timeout(services.permission_timeout, move |cancellation| {
                if cancellation.is_cancelled() {
                    return Err(RuntimeError::Cancelled);
                }
                let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
                services.ensure_running()?;
                services.request_directory(locator, rights, weak, &cancellation)
            })
    }

    /// Dispatches a registered custom operation through permission and quotas.
    ///
    /// # Errors
    ///
    /// Rejects shutdown state or bounded host-queue saturation.
    pub fn invoke(
        &self,
        name: impl Into<String>,
        version: u32,
        input: Vec<u8>,
        requested_scope: Vec<u8>,
        requested_rights: u64,
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let name = name.into();
        let timeout = Duration::from_nanos(
            services
                .operations
                .get(&(name.clone(), version))
                .ok_or(RuntimeError::UnknownOperation)?
                .limits
                .timeout_nanoseconds,
        );
        let weak = self.services.clone();
        services
            .host
            .submit_with_timeout(timeout, move |cancellation| {
                let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
                services.invoke(
                    &name,
                    version,
                    input,
                    requested_scope,
                    requested_rights,
                    cancellation,
                )
            })
    }

    /// Exports one live directory using configured authenticated persistence.
    ///
    /// # Errors
    ///
    /// Rejects foreign/closed resources, unavailable persistence, shutdown, or saturation.
    pub fn export_directory_permission(
        &self,
        directory: &Directory,
        expires_at: u64,
        now: u64,
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        if !Weak::ptr_eq(&self.services, &directory.services) || directory.is_closed() {
            return Err(RuntimeError::InvalidResource);
        }
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let weak = self.services.clone();
        let handle = directory.handle;
        let locator = directory.locator.clone();
        let rights = directory.rights;
        let quota = directory.quota;
        services.host.submit(move |cancellation| {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
            services.ensure_running()?;
            let persistence = services
                .persistence
                .as_ref()
                .ok_or(RuntimeError::PersistenceUnavailable)?;
            persistence
                .export_directory(&services, handle, &locator, rights, quota, expires_at, now)
        })
    }

    /// Imports and reopens authenticated directory authority asynchronously.
    ///
    /// # Errors
    ///
    /// Rejects unavailable persistence, shutdown, invalid policy, or saturation.
    pub fn import_directory_permission(
        &self,
        blob: Vec<u8>,
        requested_rights: FilesystemRights,
        requested_quota: u64,
        now: u64,
    ) -> Result<RuntimeTask<Directory>, RuntimeError> {
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let weak = self.services.clone();
        services.host.submit(move |cancellation| {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
            services.ensure_running()?;
            let persistence = services
                .persistence
                .as_ref()
                .ok_or(RuntimeError::PersistenceUnavailable)?;
            persistence.import_directory(
                &services,
                Weak::clone(&weak),
                &blob,
                requested_rights,
                requested_quota,
                now,
            )
        })
    }
}

struct Control {
    state: Mutex<RuntimeState>,
    audit: Mutex<VecDeque<AuditEvent>>,
    next_audit_sequence: AtomicU64,
    maximum_audit_events: usize,
}

impl Control {
    fn record(
        &self,
        category: AuditCategory,
        outcome: AuditOutcome,
        operation: &str,
        request_id: Option<u64>,
    ) {
        let Ok(mut audit) = self.audit.lock() else {
            return;
        };
        if audit.len() == self.maximum_audit_events {
            audit.pop_front();
        }
        audit.push_back(AuditEvent {
            sequence: self.next_audit_sequence.fetch_add(1, Ordering::Relaxed),
            category,
            outcome,
            operation: operation.to_string(),
            request_id,
        });
    }
}

pub(crate) struct Services {
    control: Arc<Control>,
    filesystem: Filesystem,
    filesystem_completions: Arc<FilesystemCompletions>,
    host: HostExecutor,
    registry: OpRegistry,
    operations: HashMap<(String, u32), RegisteredHostOp>,
    permission_callback: Option<Arc<dyn PermissionCallback>>,
    persistence: Option<persistence::PersistenceState>,
    next_request: AtomicU64,
    live_resources: Mutex<HashSet<ResourceHandle>>,
    shutting_down: AtomicBool,
    permission_timeout: Duration,
}

impl Services {
    fn ensure_running(&self) -> Result<(), RuntimeError> {
        if self.shutting_down.load(Ordering::Acquire) {
            return Err(RuntimeError::ShuttingDown);
        }
        let state = self
            .control
            .state
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        if *state != RuntimeState::Running {
            return Err(match *state {
                RuntimeState::Configured => RuntimeError::InvalidState,
                _ => RuntimeError::ShuttingDown,
            });
        }
        Ok(())
    }

    fn next_request_id(&self) -> Result<u64, RuntimeError> {
        let mut current = self.next_request.load(Ordering::Relaxed);
        loop {
            let next = current.checked_add(1).ok_or(RuntimeError::QuotaExceeded)?;
            match self.next_request.compare_exchange_weak(
                current,
                next,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return Ok(current),
                Err(observed) => current = observed,
            }
        }
    }

    fn request_directory(
        self: &Arc<Self>,
        locator: PathBuf,
        rights: FilesystemRights,
        weak: Weak<Self>,
        cancellation: &task::Cancellation,
    ) -> Result<Directory, RuntimeError> {
        let request_id = self.next_request_id()?;
        let locator_text = locator.to_str().ok_or(RuntimeError::InvalidArgument)?;
        let scope = locator_text.as_bytes().to_vec();
        let request = PermissionRequest {
            request_id,
            operation: DIRECTORY_OPERATION.to_string(),
            kind: DIRECTORY_PERMISSION.to_string(),
            requested_scope: scope.clone(),
            requested_rights: rights.bits(),
            requested_directory: Some(locator_text.to_string()),
            reason: Some("Host requested directory access".to_string()),
        };
        let decision = self.resolve_permission(request, cancellation)?;
        if cancellation.is_cancelled() {
            self.control.record(
                AuditCategory::DirectoryPermission,
                AuditOutcome::Cancelled,
                DIRECTORY_OPERATION,
                Some(request_id),
            );
            return Err(RuntimeError::Cancelled);
        }
        self.ensure_running()?;
        let PermissionDecision::Allow {
            rights: granted_rights,
            quota,
        } = decision
        else {
            self.record_denied(
                DIRECTORY_OPERATION,
                request_id,
                AuditCategory::DirectoryPermission,
            );
            return Err(RuntimeError::PermissionDenied);
        };
        if granted_rights == 0 || granted_rights & !rights.bits() != 0 || quota == 0 {
            self.record_denied(
                DIRECTORY_OPERATION,
                request_id,
                AuditCategory::DirectoryPermission,
            );
            return Err(RuntimeError::PermissionDenied);
        }
        let granted_rights =
            FilesystemRights::from_bits(granted_rights).map_err(RuntimeError::from_filesystem)?;
        let approved = ApprovedDirectory::from_trusted_approval(locator, granted_rights)
            .map_err(RuntimeError::from_filesystem)?;
        let handle = self
            .filesystem
            .open_approved_directory(&approved)
            .map_err(RuntimeError::from_filesystem)?;
        self.track_resource(handle)?;
        if let Err(error) = self.ensure_running() {
            let _ = self.close_resource(handle);
            return Err(error);
        }
        self.control.record(
            AuditCategory::DirectoryPermission,
            AuditOutcome::Allowed,
            DIRECTORY_OPERATION,
            Some(request_id),
        );
        Ok(Directory::new(
            weak,
            handle,
            scope,
            granted_rights.bits(),
            quota,
        ))
    }

    fn invoke(
        &self,
        name: &str,
        version: u32,
        input: Vec<u8>,
        requested_scope: Vec<u8>,
        requested_rights: u64,
        cancellation: task::Cancellation,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.ensure_running()?;
        let request_id = self.next_request_id()?;
        let operation = self
            .operations
            .get(&(name.to_owned(), version))
            .ok_or(RuntimeError::UnknownOperation)?;
        let request = PermissionRequest {
            request_id,
            operation: name.to_string(),
            kind: operation.permission.clone(),
            requested_scope: requested_scope.clone(),
            requested_rights,
            requested_directory: None,
            reason: None,
        };
        let decision = self.resolve_permission(request, &cancellation)?;
        self.ensure_running()?;
        let PermissionDecision::Allow { rights, quota } = decision else {
            self.record_denied(name, request_id, AuditCategory::CustomOperation);
            return Err(RuntimeError::PermissionDenied);
        };
        if rights != requested_rights || quota == 0 {
            self.record_denied(name, request_id, AuditCategory::CustomOperation);
            return Err(RuntimeError::PermissionDenied);
        }
        let context = DispatchContext {
            request_id,
            requested_scope,
            requested_rights,
        };
        let prepared = self
            .registry
            .prepare_dispatch(name, version, input.clone(), &context, &Allow)
            .map_err(RuntimeError::from_dispatch)?;
        if prepared.execution() != OpExecution::HostExecutor
            || prepared.start() != HostOpDisposition::Pending
        {
            return Err(RuntimeError::Internal);
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let handler_cancellation = cancellation.clone();
        let output = operation.handler.invoke(
            HostOpContext {
                request_id,
                operation: operation.operation.clone(),
                permission: operation.permission.clone(),
                cancellation,
            },
            input,
        )?;
        if handler_cancellation.is_cancelled() {
            self.control.record(
                AuditCategory::CustomOperation,
                AuditOutcome::Cancelled,
                name,
                Some(request_id),
            );
            return Err(RuntimeError::Cancelled);
        }
        self.ensure_running()?;
        if output.len() > operation.limits.max_output_bytes as usize {
            return Err(RuntimeError::OutputTooLarge);
        }
        self.control.record(
            AuditCategory::CustomOperation,
            AuditOutcome::Succeeded,
            name,
            Some(request_id),
        );
        Ok(output)
    }

    fn resolve_permission(
        &self,
        request: PermissionRequest,
        cancellation: &task::Cancellation,
    ) -> Result<PermissionDecision, RuntimeError> {
        let Some(callback) = &self.permission_callback else {
            return Err(RuntimeError::PermissionDenied);
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let responder = PermissionResponder {
            sender: Some(sender),
        };
        let callback_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            callback.request_permission(request, responder);
        }));
        if callback_result.is_err() || cancellation.is_cancelled() {
            return Err(RuntimeError::PermissionDenied);
        }
        match receiver.recv_timeout(self.permission_timeout) {
            Ok(decision) if !cancellation.is_cancelled() => Ok(decision),
            Ok(_) => Err(RuntimeError::Cancelled),
            Err(mpsc::RecvTimeoutError::Timeout) => Err(RuntimeError::TimedOut),
            Err(mpsc::RecvTimeoutError::Disconnected) => Err(RuntimeError::PermissionDenied),
        }
    }

    pub(crate) fn track_resource(&self, handle: ResourceHandle) -> Result<(), RuntimeError> {
        if self.shutting_down.load(Ordering::Acquire) {
            let _ = self.filesystem.close(handle);
            return Err(RuntimeError::ShuttingDown);
        }
        let mut resources = self
            .live_resources
            .lock()
            .map_err(|_| RuntimeError::Internal)?;
        if self.shutting_down.load(Ordering::Acquire) {
            drop(resources);
            let _ = self.filesystem.close(handle);
            return Err(RuntimeError::ShuttingDown);
        }
        resources.insert(handle);
        Ok(())
    }

    pub(crate) fn close_resource(&self, handle: ResourceHandle) -> Result<(), RuntimeError> {
        let removed = self
            .live_resources
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .remove(&handle);
        if !removed {
            return Err(RuntimeError::ResourceClosed);
        }
        self.filesystem
            .close(handle)
            .map_err(RuntimeError::from_filesystem)
    }

    fn close_all_resources(&self) -> Result<(), RuntimeError> {
        let handles: Vec<_> = self
            .live_resources
            .lock()
            .map_err(|_| RuntimeError::Internal)?
            .drain()
            .collect();
        for handle in handles {
            let _ = self.filesystem.close(handle);
        }
        Ok(())
    }

    pub(crate) fn wait_for_filesystem(&self, request_id: u64) -> Result<Vec<u8>, RuntimeError> {
        loop {
            if let Some(result) = self.filesystem_completions.take(request_id)? {
                return result.map_err(RuntimeError::from_worker);
            }
            {
                let _drain = self.filesystem_completions.drain_guard()?;
                while let Some(completion) = self
                    .filesystem
                    .next_completion()
                    .map_err(RuntimeError::from_filesystem)?
                {
                    self.filesystem_completions.store(completion)?;
                }
            }
            if let Some(result) = self.filesystem_completions.take(request_id)? {
                return result.map_err(RuntimeError::from_worker);
            }
            self.filesystem_completions.wait_signal()?;
            self.ensure_running()?;
        }
    }

    fn record_denied(&self, operation: &str, request_id: u64, category: AuditCategory) {
        self.control
            .record(category, AuditOutcome::Denied, operation, Some(request_id));
    }
}

struct Allow;

impl PermissionAuthorizer for Allow {
    fn authorize(&self, _permission_kind: &str, _context: &DispatchContext) -> bool {
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    InvalidConfiguration,
    InvalidState,
    InvalidArgument,
    DuplicateOperation,
    ReservedName,
    UnknownOperation,
    PermissionDenied,
    InputTooLarge,
    OutputTooLarge,
    QuotaExceeded,
    InvalidResource,
    ResourceClosed,
    Cancelled,
    TimedOut,
    NotSupported,
    Io,
    Protocol,
    ShuttingDown,
    HostFailed,
    PersistenceUnavailable,
    InvalidPermission,
    Internal,
}

impl RuntimeError {
    fn from_registration(error: RegistrationError) -> Self {
        match error {
            RegistrationError::InvalidDescriptor => Self::InvalidArgument,
            RegistrationError::Duplicate => Self::DuplicateOperation,
            RegistrationError::ReservedName => Self::ReservedName,
        }
    }

    fn from_dispatch(error: DispatchError) -> Self {
        match error {
            DispatchError::UnknownOp => Self::UnknownOperation,
            DispatchError::InvalidRequest => Self::InvalidArgument,
            DispatchError::InputTooLarge => Self::InputTooLarge,
            DispatchError::PermissionDenied => Self::PermissionDenied,
        }
    }

    pub(crate) fn from_filesystem(error: FilesystemError) -> Self {
        match error {
            FilesystemError::InvalidPath => Self::InvalidArgument,
            FilesystemError::InvalidResource => Self::InvalidResource,
            FilesystemError::PermissionDenied
            | FilesystemError::SymlinkDenied
            | FilesystemError::InvalidIdentity
            | FilesystemError::IdentityMismatch => Self::PermissionDenied,
            FilesystemError::ResourceClosed => Self::ResourceClosed,
            FilesystemError::WrongKind | FilesystemError::NotSupported => Self::NotSupported,
            FilesystemError::QuotaExceeded => Self::QuotaExceeded,
            FilesystemError::Cancelled => Self::Cancelled,
            FilesystemError::Protocol => Self::Protocol,
            FilesystemError::ShuttingDown => Self::ShuttingDown,
            FilesystemError::NotFound | FilesystemError::AlreadyExists | FilesystemError::Io => {
                Self::Io
            }
            FilesystemError::Internal => Self::Internal,
        }
    }

    fn from_worker(error: WorkerError) -> Self {
        match error {
            WorkerError::Cancelled => Self::Cancelled,
            WorkerError::QueueFull | WorkerError::QuotaExceeded => Self::QuotaExceeded,
            WorkerError::ShuttingDown => Self::ShuttingDown,
            WorkerError::InvalidArgument => Self::InvalidArgument,
            WorkerError::InvalidResource => Self::InvalidResource,
            WorkerError::PermissionDenied => Self::PermissionDenied,
            WorkerError::ResourceClosed => Self::ResourceClosed,
            WorkerError::NotSupported => Self::NotSupported,
            WorkerError::Io => Self::Io,
            WorkerError::Protocol => Self::Protocol,
            WorkerError::WorkerFailed => Self::Internal,
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for RuntimeError {}

fn random_nonzero_u64() -> Result<u64, RuntimeError> {
    loop {
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| RuntimeError::Internal)?;
        let value = u64::from_ne_bytes(bytes);
        if value != 0 {
            return Ok(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_permission::{PermissionCodec, PermissionCodecError};
    use std::fs;
    use std::thread;
    use std::time::Duration;

    fn temporary_directory(label: &str) -> PathBuf {
        let base = std::env::temp_dir().canonicalize().expect("temporary root");
        let path = base.join(format!(
            "zintl-runtime-embed-{label}-{}-{}",
            std::process::id(),
            random_nonzero_u64().expect("random")
        ));
        fs::create_dir(&path).expect("temporary directory");
        path
    }

    #[allow(clippy::needless_pass_by_value)]
    fn allowing_callback(request: PermissionRequest, responder: PermissionResponder) {
        responder
            .respond(PermissionDecision::Allow {
                rights: request.requested_rights,
                quota: 64 * 1_024,
            })
            .expect("live permission response");
    }

    #[test]
    // Verifies builder/start/invoke/shutdown form a complete permission-guarded host lifecycle.
    fn custom_operation_lifecycle_is_available_from_rust() {
        let runtime = RuntimeBuilder::new()
            .permission_callback(allowing_callback)
            .register_op(
                "dev.zintl.test.reverse",
                1,
                OpLimits::new(64, 64, 1_000_000_000).expect("limits"),
                "dev.zintl.permission.reverse",
                |_context: HostOpContext, mut input: Vec<u8>| {
                    input.reverse();
                    Ok(input)
                },
            )
            .expect("registered")
            .build()
            .expect("runtime");
        assert_eq!(runtime.lifecycle(), Lifecycle::Configured);
        runtime.start().expect("start");
        let output = runtime
            .invoke("dev.zintl.test.reverse", 1, b"Zintl".to_vec(), vec![], 1)
            .expect("submitted")
            .wait()
            .expect("completed");
        assert_eq!(output, b"ltniZ");
        runtime.shutdown().expect("shutdown");
        assert_eq!(runtime.lifecycle(), Lifecycle::Terminated);
    }

    #[test]
    // Verifies typed Rust directory/file APIs preserve scope, rights, opacity, and close semantics.
    fn typed_filesystem_api_is_scoped_and_attenuated() {
        let root = temporary_directory("filesystem");
        fs::write(root.join("sample.txt"), b"runtime").expect("sample");
        let runtime = RuntimeBuilder::new()
            .permission_callback(allowing_callback)
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        let directory = runtime
            .request_directory(
                &root,
                FilesystemRights::READ.union(FilesystemRights::METADATA),
            )
            .expect("submitted")
            .wait()
            .expect("directory");
        let file = directory
            .open_file(
                "sample.txt",
                FilesystemRights::READ.union(FilesystemRights::METADATA),
                false,
                false,
            )
            .expect("open submitted")
            .wait()
            .expect("file");
        assert_eq!(
            file.read(64).expect("read submitted").wait().expect("read"),
            b"runtime"
        );
        assert_eq!(
            file.stat()
                .expect("stat submitted")
                .wait()
                .expect("stat")
                .size,
            7
        );
        assert!(matches!(
            directory
                .open_file("../outside", FilesystemRights::READ, false, false)
                .expect("submitted")
                .wait(),
            Err(RuntimeError::InvalidArgument)
        ));
        file.close().expect("file close");
        directory.close().expect("directory close");
        runtime.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    // Verifies an asynchronous one-shot callback may attenuate rights but cannot broaden scope.
    fn permission_callback_is_one_shot_and_attenuating() {
        let root = temporary_directory("callback-attenuation");
        let runtime = RuntimeBuilder::new()
            .permission_callback(
                |_request: PermissionRequest, responder: PermissionResponder| {
                    thread::spawn(move || {
                        responder
                            .respond(PermissionDecision::Allow {
                                rights: FilesystemRights::READ.bits(),
                                quota: 1024,
                            })
                            .expect("live response");
                    });
                },
            )
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        let directory = runtime
            .request_directory(
                &root,
                FilesystemRights::READ.union(FilesystemRights::METADATA),
            )
            .expect("submitted")
            .wait()
            .expect("directory");
        assert_eq!(directory.rights(), FilesystemRights::READ.bits());
        runtime.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    // Verifies a panicking permission callback fails closed without unwinding across the runtime.
    fn permission_callback_panic_denies_authority() {
        let root = temporary_directory("callback-panic");
        let runtime = RuntimeBuilder::new()
            .permission_callback(
                |_request: PermissionRequest, _responder: PermissionResponder| {
                    panic!("untrusted callback panic");
                },
            )
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        assert!(matches!(
            runtime
                .request_directory(&root, FilesystemRights::READ)
                .expect("submitted")
                .wait(),
            Err(RuntimeError::PermissionDenied)
        ));
        runtime.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    // Verifies a missing resolver denies before any custom handler is invoked.
    fn permission_is_deny_by_default() {
        let runtime = RuntimeBuilder::new()
            .register_op(
                "dev.zintl.test.echo",
                1,
                OpLimits::new(8, 8, 1_000_000_000).expect("limits"),
                "dev.zintl.permission.echo",
                |_context: HostOpContext, input: Vec<u8>| Ok(input),
            )
            .expect("registered")
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        assert_eq!(
            runtime
                .invoke("dev.zintl.test.echo", 1, vec![1], vec![], 1)
                .expect("submitted")
                .wait(),
            Err(RuntimeError::PermissionDenied)
        );
        runtime.shutdown().expect("shutdown");
    }

    #[test]
    // Verifies a non-cooperative host callback settles its public task at the declared timeout.
    fn custom_operation_timeout_is_enforced_outside_the_handler() {
        let runtime = RuntimeBuilder::new()
            .permission_callback(allowing_callback)
            .register_op(
                "dev.zintl.test.slow",
                1,
                OpLimits::new(8, 8, 5_000_000).expect("limits"),
                "dev.zintl.permission.slow",
                |_context: HostOpContext, input: Vec<u8>| {
                    thread::sleep(Duration::from_millis(50));
                    Ok(input)
                },
            )
            .expect("registered")
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        assert_eq!(
            runtime
                .invoke("dev.zintl.test.slow", 1, vec![1], vec![], 1)
                .expect("submitted")
                .wait(),
            Err(RuntimeError::TimedOut)
        );
        runtime.shutdown().expect("shutdown");
    }

    #[test]
    // Verifies shutdown racing a permission callback cannot mint late directory authority.
    fn shutdown_prevents_late_permission_authority() {
        let root = temporary_directory("shutdown-permission");
        let runtime = RuntimeBuilder::new()
            .permission_callback(
                |request: PermissionRequest, responder: PermissionResponder| {
                    thread::sleep(Duration::from_millis(25));
                    let _ = responder.respond(PermissionDecision::Allow {
                        rights: request.requested_rights,
                        quota: 64 * 1_024,
                    });
                },
            )
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        let task = runtime
            .request_directory(&root, FilesystemRights::READ)
            .expect("submitted");
        runtime.shutdown().expect("shutdown");
        assert!(matches!(
            task.wait(),
            Err(RuntimeError::ShuttingDown | RuntimeError::Cancelled)
        ));
        fs::remove_dir_all(root).expect("cleanup");
    }

    struct TestCodec;

    impl PermissionCodec for TestCodec {
        fn seal(&self, envelope: &[u8]) -> Result<Vec<u8>, PermissionCodecError> {
            let mut output = envelope.to_vec();
            output.extend_from_slice(&test_checksum(envelope).to_be_bytes());
            Ok(output)
        }

        fn open(&self, blob: &[u8]) -> Result<Vec<u8>, PermissionCodecError> {
            let split = blob
                .len()
                .checked_sub(8)
                .ok_or(PermissionCodecError::Invalid)?;
            let (envelope, checksum) = blob.split_at(split);
            let checksum = u64::from_be_bytes(
                checksum
                    .try_into()
                    .map_err(|_| PermissionCodecError::Invalid)?,
            );
            if checksum != test_checksum(envelope) {
                return Err(PermissionCodecError::Invalid);
            }
            Ok(envelope.to_vec())
        }
    }

    fn test_checksum(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
    }

    #[test]
    // Verifies Rust export/import authenticates policy, reopens identity, and consumes replay once.
    fn permission_persistence_round_trips_through_rust_api() {
        let root = temporary_directory("persistence");
        fs::write(root.join("saved.txt"), b"saved").expect("sample");
        let persistence = PersistenceConfiguration::new(
            "dev.zintl.test",
            "dev.zintl.test",
            Arc::new(TestCodec),
            Arc::new(ExactPathScopeCodec),
            8,
        )
        .expect("persistence");
        let runtime = RuntimeBuilder::new()
            .permission_callback(allowing_callback)
            .permission_persistence(persistence)
            .build()
            .expect("runtime");
        runtime.start().expect("start");
        let rights = FilesystemRights::READ.union(FilesystemRights::METADATA);
        let directory = runtime
            .request_directory(&root, rights)
            .expect("request")
            .wait()
            .expect("directory");
        let blob = runtime
            .export_directory_permission(&directory, 1_000, 10)
            .expect("export")
            .wait()
            .expect("blob");
        directory.close().expect("close");
        let imported = runtime
            .import_directory_permission(blob.clone(), rights, 64 * 1_024, 11)
            .expect("import")
            .wait()
            .expect("imported directory");
        let file = imported
            .open_file("saved.txt", FilesystemRights::READ, false, false)
            .expect("open")
            .wait()
            .expect("file");
        assert_eq!(
            file.read(16).expect("read").wait().expect("bytes"),
            b"saved"
        );
        assert!(matches!(
            runtime
                .import_directory_permission(blob, rights, 64 * 1_024, 12)
                .expect("replay submitted")
                .wait(),
            Err(RuntimeError::InvalidPermission)
        ));
        runtime.shutdown().expect("shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    // Verifies a typed directory from one Rust runtime is rejected by another runtime API.
    fn cross_runtime_directory_is_rejected_before_persistence() {
        let root = temporary_directory("cross-runtime");
        let make_runtime = || {
            RuntimeBuilder::new()
                .permission_callback(allowing_callback)
                .permission_persistence(
                    PersistenceConfiguration::new(
                        "dev.zintl.test",
                        "dev.zintl.test",
                        Arc::new(TestCodec),
                        Arc::new(ExactPathScopeCodec),
                        8,
                    )
                    .expect("persistence"),
                )
                .build()
                .expect("runtime")
        };
        let first = make_runtime();
        let second = make_runtime();
        first.start().expect("first start");
        second.start().expect("second start");
        let directory = first
            .request_directory(&root, FilesystemRights::READ)
            .expect("request")
            .wait()
            .expect("directory");
        assert!(matches!(
            second.export_directory_permission(&directory, 1_000, 10),
            Err(RuntimeError::InvalidResource)
        ));
        first.shutdown().expect("first shutdown");
        second.shutdown().expect("second shutdown");
        fs::remove_dir_all(root).expect("cleanup");
    }
}
