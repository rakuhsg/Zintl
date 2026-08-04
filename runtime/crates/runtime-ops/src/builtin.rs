//! Stable builtin operation catalog shared by every engine adapter.

use crate::{
    HostOpDisposition, OpDescriptor, OpExecution, OpHandler, OpRegistryBuilder, RegistrationError,
    ValidatedOpContext,
};

pub const BUILTIN_SCHEMA_VERSION: u32 = 1;
pub const TIMER_SLEEP_ID: u32 = 2;
pub const FS_READ_FILE_ID: u32 = 17;
pub const FS_WRITE_FILE_ID: u32 = 18;
pub const FS_CREATE_DIRECTORY_ID: u32 = 19;
pub const FS_LIST_DIRECTORY_ID: u32 = 20;
pub const FS_METADATA_ID: u32 = 21;
pub const FS_REMOVE_FILE_ID: u32 = 22;
pub const FS_REMOVE_DIRECTORY_ID: u32 = 23;
pub const FS_RENAME_ID: u32 = 24;
pub const FS_OPEN_RELATIVE_ID: u32 = 25;
pub const RESOURCE_CLOSE_ID: u32 = 32;

const KIB: u32 = 1_024;
const MIB: u32 = 1_024 * KIB;
const SECOND_TICKS: u64 = 1_000_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuiltinKind {
    TimerSleep,
    FsReadFile,
    FsWriteFile,
    FsCreateDirectory,
    FsListDirectory,
    FsMetadata,
    FsRemoveFile,
    FsRemoveDirectory,
    FsRename,
    FsOpenRelative,
    ResourceClose,
}

impl BuiltinKind {
    #[must_use]
    pub const fn stable_id(self) -> u32 {
        match self {
            Self::TimerSleep => TIMER_SLEEP_ID,
            Self::FsReadFile => FS_READ_FILE_ID,
            Self::FsWriteFile => FS_WRITE_FILE_ID,
            Self::FsCreateDirectory => FS_CREATE_DIRECTORY_ID,
            Self::FsListDirectory => FS_LIST_DIRECTORY_ID,
            Self::FsMetadata => FS_METADATA_ID,
            Self::FsRemoveFile => FS_REMOVE_FILE_ID,
            Self::FsRemoveDirectory => FS_REMOVE_DIRECTORY_ID,
            Self::FsRename => FS_RENAME_ID,
            Self::FsOpenRelative => FS_OPEN_RELATIVE_ID,
            Self::ResourceClose => RESOURCE_CLOSE_ID,
        }
    }
}

#[derive(Clone, Copy)]
struct BuiltinHandler;

impl OpHandler for BuiltinHandler {
    fn start(&self, _context: ValidatedOpContext, _input: Vec<u8>) -> HostOpDisposition {
        HostOpDisposition::Pending
    }
}

struct BuiltinSpec {
    kind: BuiltinKind,
    name: &'static str,
    max_input_bytes: u32,
    max_output_bytes: u32,
    permission: &'static str,
    execution: OpExecution,
    timeout_ticks: u64,
}

const BUILTINS: [BuiltinSpec; 11] = [
    BuiltinSpec {
        kind: BuiltinKind::TimerSleep,
        name: "zintl.builtin.timer.sleep",
        max_input_bytes: 4,
        max_output_bytes: 1,
        permission: "zintl.permission.timer.use",
        execution: OpExecution::Runtime,
        timeout_ticks: 86_400 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsReadFile,
        name: "zintl.builtin.fs.read-file",
        max_input_bytes: 8 * KIB,
        max_output_bytes: 4 * MIB,
        permission: "zintl.permission.fs.read",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsWriteFile,
        name: "zintl.builtin.fs.write-file",
        max_input_bytes: 4 * MIB,
        max_output_bytes: 1,
        permission: "zintl.permission.fs.write",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsCreateDirectory,
        name: "zintl.builtin.fs.create-directory",
        max_input_bytes: 8 * KIB,
        max_output_bytes: 1,
        permission: "zintl.permission.fs.create",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsListDirectory,
        name: "zintl.builtin.fs.list-directory",
        max_input_bytes: 8 * KIB,
        max_output_bytes: MIB,
        permission: "zintl.permission.fs.enumerate",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsMetadata,
        name: "zintl.builtin.fs.metadata",
        max_input_bytes: 8 * KIB,
        max_output_bytes: 16,
        permission: "zintl.permission.fs.metadata",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsRemoveFile,
        name: "zintl.builtin.fs.remove-file",
        max_input_bytes: 8 * KIB,
        max_output_bytes: 1,
        permission: "zintl.permission.fs.write",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsRemoveDirectory,
        name: "zintl.builtin.fs.remove-directory",
        max_input_bytes: 8 * KIB,
        max_output_bytes: 1,
        permission: "zintl.permission.fs.write",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsRename,
        name: "zintl.builtin.fs.rename",
        max_input_bytes: 16 * KIB,
        max_output_bytes: 1,
        permission: "zintl.permission.fs.write",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::FsOpenRelative,
        name: "zintl.builtin.fs.open-relative",
        max_input_bytes: 8 * KIB,
        max_output_bytes: 64,
        permission: "zintl.permission.fs.open",
        execution: OpExecution::FilesystemWorker,
        timeout_ticks: 30 * SECOND_TICKS,
    },
    BuiltinSpec {
        kind: BuiltinKind::ResourceClose,
        name: "zintl.builtin.resource.close",
        max_input_bytes: 32,
        max_output_bytes: 1,
        permission: "zintl.permission.resource.close",
        execution: OpExecution::Runtime,
        timeout_ticks: SECOND_TICKS,
    },
];

/// Registers the complete builtin catalog before registry freeze.
///
/// # Errors
///
/// Returns a registration error only for an internal duplicate or invalid
/// catalog descriptor, which startup must treat as fatal configuration.
pub fn register_all(builder: &mut OpRegistryBuilder) -> Result<(), RegistrationError> {
    for spec in BUILTINS {
        builder.register_builtin(
            OpDescriptor::new(
                spec.kind.stable_id(),
                spec.name,
                1,
                BUILTIN_SCHEMA_VERSION,
                spec.max_input_bytes,
                spec.max_output_bytes,
                spec.permission,
                spec.execution,
                spec.timeout_ticks,
            )?,
            BuiltinHandler,
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{BUILTIN_SCHEMA_VERSION, BUILTINS, register_all};
    use crate::{
        DispatchContext, DispatchError, HostOpDisposition, OpDescriptor, OpExecution, OpHandler,
        OpRegistryBuilder, PermissionAuthorizer, ValidatedOpContext,
    };

    struct Authorizer(bool);

    impl PermissionAuthorizer for Authorizer {
        fn authorize(&self, _permission_kind: &str, _context: &DispatchContext) -> bool {
            self.0
        }
    }

    struct CustomHandler;

    impl OpHandler for CustomHandler {
        fn start(&self, _context: ValidatedOpContext, input: Vec<u8>) -> HostOpDisposition {
            HostOpDisposition::Completed(input)
        }
    }

    #[test]
    // Verifies stable builtin identity/name/version snapshots cannot drift accidentally.
    fn catalog_identity_snapshot_is_stable() {
        assert_eq!(BUILTIN_SCHEMA_VERSION, 1);
        assert_eq!(
            BUILTINS
                .iter()
                .map(|spec| (spec.kind.stable_id(), spec.name))
                .collect::<Vec<_>>(),
            vec![
                (2, "zintl.builtin.timer.sleep"),
                (17, "zintl.builtin.fs.read-file"),
                (18, "zintl.builtin.fs.write-file"),
                (19, "zintl.builtin.fs.create-directory"),
                (20, "zintl.builtin.fs.list-directory"),
                (21, "zintl.builtin.fs.metadata"),
                (22, "zintl.builtin.fs.remove-file"),
                (23, "zintl.builtin.fs.remove-directory"),
                (24, "zintl.builtin.fs.rename"),
                (25, "zintl.builtin.fs.open-relative"),
                (32, "zintl.builtin.resource.close"),
            ]
        );
    }

    #[test]
    // Verifies every builtin freezes into one registry with mandatory bounded metadata.
    fn complete_catalog_registers_with_bounds() {
        let mut builder = OpRegistryBuilder::new();
        register_all(&mut builder).expect("valid catalog");
        let registry = builder.freeze();
        assert_eq!(registry.len(), BUILTINS.len());
        for spec in BUILTINS {
            let descriptor = registry.descriptor(spec.name, 1).expect("descriptor");
            assert_eq!(descriptor.stable_id(), spec.kind.stable_id());
            assert_eq!(descriptor.schema_version(), BUILTIN_SCHEMA_VERSION);
            assert!(descriptor.max_input_bytes() > 0);
            assert!(descriptor.max_output_bytes() > 0);
            assert!(descriptor.timeout_ticks() > 0);
        }
    }

    #[test]
    // Verifies builtin dispatch uses the same authorization-first preparation as custom ops.
    fn builtin_dispatch_is_authorization_first() {
        let mut builder = OpRegistryBuilder::new();
        register_all(&mut builder).expect("catalog");
        let registry = builder.freeze();
        let context = DispatchContext {
            request_id: 1,
            requested_scope: Vec::new(),
            requested_rights: 1,
        };
        assert!(matches!(
            registry.prepare_dispatch(
                "zintl.builtin.timer.sleep",
                1,
                vec![0, 0, 0, 1],
                &context,
                &Authorizer(false)
            ),
            Err(DispatchError::PermissionDenied)
        ));
        let prepared = registry
            .prepare_dispatch(
                "zintl.builtin.timer.sleep",
                1,
                vec![0, 0, 0, 1],
                &context,
                &Authorizer(true),
            )
            .expect("authorized");
        assert_eq!(prepared.start(), HostOpDisposition::Pending);
    }

    #[test]
    // Verifies builtin and custom descriptors return identical policy and input-limit errors.
    fn custom_and_builtin_dispatch_fail_with_parity() {
        let mut builder = OpRegistryBuilder::new();
        register_all(&mut builder).expect("catalog");
        builder
            .register_custom(
                OpDescriptor::new(
                    0x8000_0001,
                    "com.example.bytes.echo",
                    1,
                    1,
                    4,
                    4,
                    "com.example.bytes.echo",
                    OpExecution::HostExecutor,
                    10,
                )
                .expect("descriptor"),
                CustomHandler,
            )
            .expect("custom op");
        let registry = builder.freeze();
        let context = DispatchContext {
            request_id: 1,
            requested_scope: Vec::new(),
            requested_rights: 1,
        };
        for name in ["zintl.builtin.timer.sleep", "com.example.bytes.echo"] {
            assert!(matches!(
                registry.prepare_dispatch(name, 1, vec![0; 4], &context, &Authorizer(false)),
                Err(DispatchError::PermissionDenied)
            ));
            assert!(matches!(
                registry.prepare_dispatch(name, 1, vec![0; 5], &context, &Authorizer(true)),
                Err(DispatchError::InputTooLarge)
            ));
        }
    }
}
