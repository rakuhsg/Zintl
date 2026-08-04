//! Typed, immutable op registry and authorization-first dispatch preparation.

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub mod builtin;

/// Execution isolation selected by a descriptor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OpExecution {
    Runtime,
    HostExecutor,
    FilesystemWorker,
}

/// Every security-relevant property is mandatory and bounded.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpDescriptor {
    stable_id: u32,
    name: String,
    version: u32,
    schema_version: u32,
    max_input_bytes: u32,
    max_output_bytes: u32,
    permission_kind: String,
    execution: OpExecution,
    timeout_ticks: u64,
}

impl OpDescriptor {
    /// Constructs a descriptor after validating every required limit.
    ///
    /// # Errors
    ///
    /// Rejects zero IDs, versions, limits, timeout, malformed names, or an
    /// empty permission kind.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        stable_id: u32,
        name: impl Into<String>,
        version: u32,
        schema_version: u32,
        max_input_bytes: u32,
        max_output_bytes: u32,
        permission_kind: impl Into<String>,
        execution: OpExecution,
        timeout_ticks: u64,
    ) -> Result<Self, RegistrationError> {
        let name = name.into();
        let permission_kind = permission_kind.into();
        if stable_id == 0
            || version == 0
            || schema_version == 0
            || max_input_bytes == 0
            || max_output_bytes == 0
            || timeout_ticks == 0
            || !valid_namespaced_name(&name)
            || !valid_namespaced_name(&permission_kind)
        {
            return Err(RegistrationError::InvalidDescriptor);
        }
        Ok(Self {
            stable_id,
            name,
            version,
            schema_version,
            max_input_bytes,
            max_output_bytes,
            permission_kind,
            execution,
            timeout_ticks,
        })
    }

    #[must_use]
    pub const fn stable_id(&self) -> u32 {
        self.stable_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[must_use]
    pub const fn max_input_bytes(&self) -> u32 {
        self.max_input_bytes
    }

    #[must_use]
    pub const fn max_output_bytes(&self) -> u32 {
        self.max_output_bytes
    }

    #[must_use]
    pub fn permission_kind(&self) -> &str {
        &self.permission_kind
    }

    #[must_use]
    pub const fn execution(&self) -> OpExecution {
        self.execution
    }

    #[must_use]
    pub const fn timeout_ticks(&self) -> u64 {
        self.timeout_ticks
    }
}

fn valid_namespaced_name(value: &str) -> bool {
    value.len() <= 192
        && value.split('.').count() >= 3
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        && !value.split('.').any(str::is_empty)
}

/// Metadata supplied to policy without any engine value or mutable resource.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DispatchContext {
    pub request_id: u64,
    pub requested_scope: Vec<u8>,
    pub requested_rights: u64,
}

/// Trusted permission check used by both builtin and custom operations.
pub trait PermissionAuthorizer {
    fn authorize(&self, permission_kind: &str, context: &DispatchContext) -> bool;
}

/// A handler receives only validated bytes, metadata, and its declared limits.
pub trait OpHandler: Send + Sync + 'static {
    fn start(&self, context: ValidatedOpContext, input: Vec<u8>) -> HostOpDisposition;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatedOpContext {
    pub request_id: u64,
    pub stable_op_id: u32,
    pub schema_version: u32,
    pub max_output_bytes: u32,
    pub timeout_ticks: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum HostOpDisposition {
    Completed(Vec<u8>),
    Pending,
    Failed,
}

struct RegisteredOp {
    descriptor: OpDescriptor,
    handler: Arc<dyn OpHandler>,
}

/// Mutable builder. `freeze` consumes it so runtime-start registries are immutable.
#[derive(Default)]
pub struct OpRegistryBuilder {
    by_key: HashMap<(String, u32), RegisteredOp>,
    stable_ids: HashSet<u32>,
}

impl OpRegistryBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a custom operation.
    ///
    /// # Errors
    ///
    /// Rejects builtin namespace use, duplicate name/version, or duplicate ID.
    pub fn register_custom<H: OpHandler>(
        &mut self,
        descriptor: OpDescriptor,
        handler: H,
    ) -> Result<(), RegistrationError> {
        if descriptor.name.starts_with("zintl.builtin.") {
            return Err(RegistrationError::ReservedName);
        }
        self.insert(descriptor, Arc::new(handler))
    }

    /// Registers an implementation-owned builtin operation.
    ///
    /// # Errors
    ///
    /// Rejects non-builtin names, duplicate name/version, or duplicate ID.
    pub fn register_builtin<H: OpHandler>(
        &mut self,
        descriptor: OpDescriptor,
        handler: H,
    ) -> Result<(), RegistrationError> {
        if !descriptor.name.starts_with("zintl.builtin.") {
            return Err(RegistrationError::ReservedName);
        }
        self.insert(descriptor, Arc::new(handler))
    }

    fn insert(
        &mut self,
        descriptor: OpDescriptor,
        handler: Arc<dyn OpHandler>,
    ) -> Result<(), RegistrationError> {
        let key = (descriptor.name.clone(), descriptor.version);
        if self.by_key.contains_key(&key) || self.stable_ids.contains(&descriptor.stable_id) {
            return Err(RegistrationError::Duplicate);
        }
        self.stable_ids.insert(descriptor.stable_id);
        self.by_key.insert(
            key,
            RegisteredOp {
                descriptor,
                handler,
            },
        );
        Ok(())
    }

    #[must_use]
    pub fn freeze(self) -> OpRegistry {
        OpRegistry {
            by_key: self.by_key,
        }
    }
}

/// Immutable registry used after runtime start.
pub struct OpRegistry {
    by_key: HashMap<(String, u32), RegisteredOp>,
}

impl OpRegistry {
    #[must_use]
    pub fn descriptor(&self, name: &str, version: u32) -> Option<&OpDescriptor> {
        self.by_key
            .get(&(name.to_owned(), version))
            .map(|operation| &operation.descriptor)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.by_key.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_key.is_empty()
    }

    /// Validates lookup, payload limit, and permission before exposing a handler.
    ///
    /// # Errors
    ///
    /// Rejects unknown operations, invalid request IDs, oversized payloads, or
    /// denied permission without invoking the handler.
    pub fn prepare_dispatch(
        &self,
        name: &str,
        version: u32,
        input: Vec<u8>,
        context: &DispatchContext,
        authorizer: &impl PermissionAuthorizer,
    ) -> Result<PreparedOp, DispatchError> {
        if context.request_id == 0 {
            return Err(DispatchError::InvalidRequest);
        }
        let operation = self
            .by_key
            .get(&(name.to_owned(), version))
            .ok_or(DispatchError::UnknownOp)?;
        if input.len() > operation.descriptor.max_input_bytes as usize {
            return Err(DispatchError::InputTooLarge);
        }
        if !authorizer.authorize(&operation.descriptor.permission_kind, context) {
            return Err(DispatchError::PermissionDenied);
        }
        Ok(PreparedOp {
            handler: operation.handler.clone(),
            execution: operation.descriptor.execution,
            max_output_bytes: operation.descriptor.max_output_bytes,
            context: ValidatedOpContext {
                request_id: context.request_id,
                stable_op_id: operation.descriptor.stable_id,
                schema_version: operation.descriptor.schema_version,
                max_output_bytes: operation.descriptor.max_output_bytes,
                timeout_ticks: operation.descriptor.timeout_ticks,
            },
            input,
        })
    }
}

/// Authorized invocation that can be moved to its declared executor.
pub struct PreparedOp {
    handler: Arc<dyn OpHandler>,
    execution: OpExecution,
    max_output_bytes: u32,
    context: ValidatedOpContext,
    input: Vec<u8>,
}

impl PreparedOp {
    #[must_use]
    pub const fn execution(&self) -> OpExecution {
        self.execution
    }

    #[must_use]
    pub fn start(self) -> HostOpDisposition {
        match self.handler.start(self.context, self.input) {
            HostOpDisposition::Completed(output)
                if output.len() > self.max_output_bytes as usize =>
            {
                HostOpDisposition::Failed
            }
            disposition => disposition,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistrationError {
    InvalidDescriptor,
    Duplicate,
    ReservedName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DispatchError {
    UnknownOp,
    InvalidRequest,
    InputTooLarge,
    PermissionDenied,
}

#[cfg(test)]
mod tests {
    use super::{
        DispatchContext, DispatchError, HostOpDisposition, OpDescriptor, OpExecution, OpHandler,
        OpRegistryBuilder, PermissionAuthorizer, RegistrationError, ValidatedOpContext,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Handler(Arc<AtomicUsize>);

    impl OpHandler for Handler {
        fn start(&self, _context: ValidatedOpContext, input: Vec<u8>) -> HostOpDisposition {
            self.0.fetch_add(1, Ordering::SeqCst);
            HostOpDisposition::Completed(input)
        }
    }

    struct Authorizer(bool);

    impl PermissionAuthorizer for Authorizer {
        fn authorize(&self, _permission_kind: &str, _context: &DispatchContext) -> bool {
            self.0
        }
    }

    fn descriptor(name: &str, id: u32) -> OpDescriptor {
        OpDescriptor::new(
            id,
            name,
            1,
            1,
            4,
            8,
            "com.example.permission.invoke",
            OpExecution::HostExecutor,
            10,
        )
        .expect("descriptor")
    }

    fn context() -> DispatchContext {
        DispatchContext {
            request_id: 1,
            requested_scope: Vec::new(),
            requested_rights: 1,
        }
    }

    #[test]
    // Verifies custom operations cannot replace the builtin namespace.
    fn custom_op_cannot_override_builtin_namespace() {
        let mut builder = OpRegistryBuilder::new();
        assert_eq!(
            builder.register_custom(
                descriptor("zintl.builtin.resource.close", 1),
                Handler(Arc::new(AtomicUsize::new(0)))
            ),
            Err(RegistrationError::ReservedName)
        );
    }

    #[test]
    // Verifies duplicate names, versions, and stable IDs are rejected.
    fn duplicate_registration_is_rejected() {
        let mut builder = OpRegistryBuilder::new();
        builder
            .register_custom(
                descriptor("com.example.image.decode", 1),
                Handler(Arc::new(AtomicUsize::new(0))),
            )
            .expect("registered");
        assert_eq!(
            builder.register_custom(
                descriptor("com.example.image.decode", 2),
                Handler(Arc::new(AtomicUsize::new(0)))
            ),
            Err(RegistrationError::Duplicate)
        );
    }

    #[test]
    // Verifies zero payload limits cannot represent an unlimited descriptor.
    fn unbounded_descriptor_is_rejected() {
        assert_eq!(
            OpDescriptor::new(
                1,
                "com.example.image.decode",
                1,
                1,
                0,
                1,
                "com.example.permission.invoke",
                OpExecution::HostExecutor,
                1
            ),
            Err(RegistrationError::InvalidDescriptor)
        );
    }

    #[test]
    // Verifies permission denial prevents a custom handler from running.
    fn custom_op_cannot_bypass_permission() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = OpRegistryBuilder::new();
        builder
            .register_custom(
                descriptor("com.example.image.decode", 1),
                Handler(calls.clone()),
            )
            .expect("registered");
        let registry = builder.freeze();
        assert!(matches!(
            registry.prepare_dispatch(
                "com.example.image.decode",
                1,
                vec![1],
                &context(),
                &Authorizer(false)
            ),
            Err(DispatchError::PermissionDenied)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    // Verifies input limits are checked before the handler runs.
    fn custom_op_cannot_bypass_input_limit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut builder = OpRegistryBuilder::new();
        builder
            .register_custom(
                descriptor("com.example.image.decode", 1),
                Handler(calls.clone()),
            )
            .expect("registered");
        let registry = builder.freeze();
        assert!(matches!(
            registry.prepare_dispatch(
                "com.example.image.decode",
                1,
                vec![0; 5],
                &context(),
                &Authorizer(true)
            ),
            Err(DispatchError::InputTooLarge)
        ));
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    // Verifies synchronous output cannot bypass the descriptor result limit.
    fn custom_op_cannot_bypass_output_limit() {
        let mut builder = OpRegistryBuilder::new();
        let descriptor = OpDescriptor::new(
            1,
            "com.example.image.decode",
            1,
            1,
            4,
            2,
            "com.example.permission.invoke",
            OpExecution::HostExecutor,
            10,
        )
        .expect("descriptor");
        builder
            .register_custom(descriptor, Handler(Arc::new(AtomicUsize::new(0))))
            .expect("registered");
        let prepared = builder
            .freeze()
            .prepare_dispatch(
                "com.example.image.decode",
                1,
                vec![0; 4],
                &context(),
                &Authorizer(true),
            )
            .expect("prepared");
        assert_eq!(prepared.start(), HostOpDisposition::Failed);
    }
}
