//! Safe, engine-neutral Rust embedding API for the Zintl runtime.
//!
//! The embedder configures finite limits, named virtual filesystems, their
//! application-owned [`Authority`] implementations, and optional custom
//! operations before starting the runtime. JavaScript engines attach through
//! [`EngineSession`], which exchanges typed events and owned bytes; it never
//! exposes an engine value, OS descriptor, native pointer, resource-table
//! handle, or host filesystem path.
//!
//! # Lifecycle
//!
//! ```no_run
//! use runtime_embed::{RuntimeBuilder, Source, VfsConfig};
//!
//! let runtime = RuntimeBuilder::new()
//!     .add_vfs(VfsConfig {
//!         name: "project".into(),
//!         source: Source::LoadDir { path: ".".into() },
//!         authority: None,
//!     })?
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
mod task;
mod vfs;

pub use engine::{DriveReport, EngineSession};
pub use runtime_engine;
pub use task::RuntimeTask;
pub use vfs::{
    Authority, AuthorizationOperation, AuthorizationRequest, AuthorizationResult, Source,
    VfsConfig, VfsDescriptor,
};

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
use runtime_resource::{ResourceHandle, ResourceOwner};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use vfs::valid_vfs_name;

const FIRST_CUSTOM_OP_ID: u32 = 1_024;
const INTERNAL_CUSTOM_OP_POLICY: &str = "zintl.internal.custom-operation";

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

#[derive(Clone)]
pub struct HostOpContext {
    pub request_id: u64,
    pub operation: String,
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
    limits: OpLimits,
    handler: Arc<dyn HostOp>,
}

struct MountedVfs {
    descriptor: VfsDescriptor,
    name: String,
    root: ResourceHandle,
    authority: Option<Arc<dyn Authority>>,
}

struct PendingHandler;

impl OpHandler for PendingHandler {
    fn start(&self, _context: ValidatedOpContext, _input: Vec<u8>) -> HostOpDisposition {
        HostOpDisposition::Pending
    }
}

pub struct RuntimeBuilder {
    configuration: RuntimeConfiguration,
    registry: OpRegistryBuilder,
    operations: HashMap<(String, u32), RegisteredHostOp>,
    next_custom_op_id: u32,
    virtual_filesystems: Vec<VfsConfig>,
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
            registry: OpRegistryBuilder::new(),
            operations: HashMap::new(),
            next_custom_op_id: FIRST_CUSTOM_OP_ID,
            virtual_filesystems: Vec::new(),
        }
    }

    #[must_use]
    pub fn configuration(mut self, configuration: RuntimeConfiguration) -> Self {
        self.configuration = configuration;
        self
    }

    /// Registers a named virtual filesystem before the runtime is built.
    ///
    /// JavaScript can address the mount only through `<name>://<relative-path>`;
    /// the real source path is never sent across the engine boundary.
    ///
    /// # Errors
    ///
    /// Rejects invalid URL-scheme names, duplicate names, or descriptor exhaustion.
    pub fn add_vfs(mut self, configuration: VfsConfig) -> Result<Self, RuntimeError> {
        if !valid_vfs_name(&configuration.name)
            || self
                .virtual_filesystems
                .iter()
                .any(|vfs| vfs.name == configuration.name)
            || self.virtual_filesystems.len() >= u32::MAX as usize
        {
            return Err(RuntimeError::InvalidConfiguration);
        }
        self.virtual_filesystems.push(configuration);
        Ok(self)
    }

    /// Registers a bounded custom byte operation before build.
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
        handler: impl HostOp,
    ) -> Result<Self, RuntimeError> {
        let name = name.into();
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
            INTERNAL_CUSTOM_OP_POLICY,
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
        let vfs_rights = FilesystemRights::READ.union(FilesystemRights::METADATA);
        let mut virtual_filesystems = HashMap::new();
        let mut initial_resources = HashSet::new();
        for (index, configuration) in self.virtual_filesystems.into_iter().enumerate() {
            let descriptor = VfsDescriptor(
                u32::try_from(index)
                    .map_err(|_| RuntimeError::InvalidConfiguration)?
                    .checked_add(1)
                    .ok_or(RuntimeError::InvalidConfiguration)?,
            );
            let Source::LoadDir { path } = configuration.source;
            let approved = ApprovedDirectory::from_trusted_approval(path, vfs_rights)
                .map_err(RuntimeError::from_filesystem)?;
            let root = filesystem
                .open_approved_directory(&approved)
                .map_err(RuntimeError::from_filesystem)?;
            initial_resources.insert(root);
            virtual_filesystems.insert(
                configuration.name.clone(),
                MountedVfs {
                    descriptor,
                    name: configuration.name,
                    root,
                    authority: configuration.authority,
                },
            );
        }
        let host = HostExecutor::new(configuration.host_workers, configuration.host_queue_limit)?;
        let control = Arc::new(Control {
            state: Mutex::new(RuntimeState::Configured),
            audit: Mutex::new(VecDeque::new()),
            next_audit_sequence: AtomicU64::new(1),
            maximum_audit_events: configuration.maximum_audit_events,
        });
        let services = Arc::new(Services {
            control: control.clone(),
            filesystem,
            filesystem_completions: notifier,
            host,
            registry: self.registry.freeze(),
            operations: self.operations,
            virtual_filesystems,
            next_request: AtomicU64::new(1),
            live_resources: Mutex::new(initial_resources),
            shutting_down: AtomicBool::new(false),
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

    /// Returns the opaque descriptor assigned to a registered VFS name.
    #[must_use]
    pub fn vfs_descriptor(&self, name: &str) -> Option<VfsDescriptor> {
        self.services()
            .ok()?
            .virtual_filesystems
            .get(name)
            .map(|vfs| vfs.descriptor)
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

    /// Reads a file addressed by a registered virtual filesystem URL.
    ///
    /// # Errors
    ///
    /// Rejects OS paths, unknown VFS names, unsafe relative paths, denied
    /// authority, invalid limits, and filesystem failures.
    pub fn read_file(
        &self,
        url: impl Into<String>,
        maximum_bytes: usize,
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        self.handle()?.read_file(url, maximum_bytes)
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
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        self.handle()?.invoke(name, version, input)
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
    /// Reads a bounded file through a registered VFS URL.
    ///
    /// The URL must use `<vfs>://<relative-path>` syntax. Absolute OS paths and
    /// path traversal are rejected before any authority is consulted.
    ///
    /// # Errors
    ///
    /// Rejects invalid URLs, unknown mounts, denied authority, queue
    /// saturation, shutdown, and filesystem failures.
    pub fn read_file(
        &self,
        url: impl Into<String>,
        maximum_bytes: usize,
    ) -> Result<RuntimeTask<Vec<u8>>, RuntimeError> {
        if maximum_bytes == 0 {
            return Err(RuntimeError::InvalidArgument);
        }
        let services = self.services.upgrade().ok_or(RuntimeError::ShuttingDown)?;
        services.ensure_running()?;
        let url = url.into();
        let weak = self.services.clone();
        services.host.submit(move |cancellation| {
            if cancellation.is_cancelled() {
                return Err(RuntimeError::Cancelled);
            }
            let services = weak.upgrade().ok_or(RuntimeError::ShuttingDown)?;
            services.read_vfs_file(&url, maximum_bytes, &cancellation)
        })
    }

    /// Dispatches a registered custom operation with its configured bounds.
    ///
    /// # Errors
    ///
    /// Rejects shutdown state or bounded host-queue saturation.
    pub fn invoke(
        &self,
        name: impl Into<String>,
        version: u32,
        input: Vec<u8>,
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
                services.invoke(&name, version, input, cancellation)
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
    virtual_filesystems: HashMap<String, MountedVfs>,
    next_request: AtomicU64,
    live_resources: Mutex<HashSet<ResourceHandle>>,
    shutting_down: AtomicBool,
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

    fn read_vfs_file(
        &self,
        url: &str,
        maximum_bytes: usize,
        cancellation: &task::Cancellation,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.ensure_running()?;
        let (name, path) = url.split_once("://").ok_or(RuntimeError::InvalidArgument)?;
        if !valid_vfs_name(name) || path.is_empty() || path.contains(['?', '#']) {
            return Err(RuntimeError::InvalidArgument);
        }
        let relative =
            runtime_filesystem::RelativePath::parse(path).map_err(RuntimeError::from_filesystem)?;
        let mounted = self
            .virtual_filesystems
            .get(name)
            .ok_or(RuntimeError::InvalidArgument)?;
        if let Some(authority) = &mounted.authority {
            let request = AuthorizationRequest {
                vfs: &mounted.name,
                path,
                operation: AuthorizationOperation::ReadFile,
            };
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                authority.authorization_requested(&request)
            }));
            if !matches!(result, Ok(AuthorizationResult::Allow)) {
                return Err(RuntimeError::PermissionDenied);
            }
        }
        if cancellation.is_cancelled() {
            return Err(RuntimeError::Cancelled);
        }
        let file = self
            .filesystem
            .open_relative(
                mounted.root,
                &relative,
                FilesystemRights::READ,
                false,
                false,
            )
            .map_err(RuntimeError::from_filesystem)?;
        self.track_resource(file)?;
        let result = (|| {
            let request_id = self.next_request_id()?;
            self.filesystem
                .submit_file_read(request_id, file, maximum_bytes)
                .map_err(RuntimeError::from_filesystem)?;
            self.wait_for_filesystem(request_id)
        })();
        let _ = self.close_resource(file);
        result
    }

    fn invoke(
        &self,
        name: &str,
        version: u32,
        input: Vec<u8>,
        cancellation: task::Cancellation,
    ) -> Result<Vec<u8>, RuntimeError> {
        self.ensure_running()?;
        let request_id = self.next_request_id()?;
        let operation = self
            .operations
            .get(&(name.to_owned(), version))
            .ok_or(RuntimeError::UnknownOperation)?;
        let context = DispatchContext {
            request_id,
            requested_scope: Vec::new(),
            requested_rights: 0,
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
