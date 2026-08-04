//! Stable, panic-contained C ABI for the embedded runtime.

use runtime_core::runtime::{Runtime, RuntimeConfig, RuntimeError};
use runtime_core::{ErrorCode, RuntimeState};
use runtime_event_loop::CompletionNotifier;
use runtime_event_loop::worker::{CancellationToken, WorkerCompletion, WorkerError};
use runtime_filesystem::{
    ApprovedDirectory, DirectoryIdentity, Filesystem, FilesystemError, FilesystemOperation,
    FilesystemRights, RelativePath,
};
use runtime_resource::{ResourceHandle, ResourceOwner};
use std::collections::HashMap;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

pub const ABI_VERSION: u32 = 1;

pub const RT_OK: u32 = 0;
pub const RT_EMPTY: u32 = 1;
pub const RT_BUFFER_TOO_SMALL: u32 = 2;
pub const RT_INVALID_ARGUMENT: u32 = 3;
pub const RT_INVALID_STATE: u32 = 4;
pub const RT_DUPLICATE_REQUEST: u32 = 5;
pub const RT_UNKNOWN_REQUEST: u32 = 6;
pub const RT_QUOTA_EXCEEDED: u32 = 7;
pub const RT_RUNTIME_SHUTTING_DOWN: u32 = 8;
pub const RT_OPERATION_FAILED: u32 = 9;
pub const RT_INTERNAL: u32 = 255;

static NEXT_RESOURCE_OWNER: AtomicU64 = AtomicU64::new(1);

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtConfig {
    pub abi_version: u32,
    pub flags: u32,
    pub max_inflight_requests: u32,
    pub max_completion_bytes: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RtStatus {
    pub status: u32,
    pub detail: u32,
}

impl RtStatus {
    const fn new(status: u32) -> Self {
        Self { status, detail: 0 }
    }
}

type NotifierCallback = extern "C" fn(*mut c_void);

#[derive(Clone, Copy)]
struct Notifier {
    callback: NotifierCallback,
    user_data: usize,
}

struct NotifierProxy(Arc<Mutex<Option<Notifier>>>);

impl CompletionNotifier for NotifierProxy {
    fn notify_drain_needed(&self) {
        let notifier = self.0.lock().ok().and_then(|guard| *guard);
        if let Some(notifier) = notifier {
            (notifier.callback)(notifier.user_data as *mut c_void);
        }
    }
}

struct FilesystemHandles {
    next_id: u64,
    maximum: usize,
    handles: HashMap<u64, ResourceHandle>,
}

#[repr(C)]
pub struct RtRuntime {
    core: Mutex<Runtime>,
    notifier: Arc<Mutex<Option<Notifier>>>,
    filesystem: Mutex<Option<Filesystem>>,
    filesystem_handles: Mutex<FilesystemHandles>,
    filesystem_cancellations: Mutex<HashMap<u64, CancellationToken>>,
    resource_owner: ResourceOwner,
    max_inflight_requests: usize,
    max_completion_bytes: usize,
}

/// Creates one opaque runtime allocated and freed by Rust.
///
/// # Safety
///
/// `config` and `out_runtime` must be aligned and valid for reads/writes for the
/// duration of this call. `out_runtime` must not alias `config`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_new(
    config: *const RtConfig,
    out_runtime: *mut *mut RtRuntime,
) -> RtStatus {
    boundary(|| {
        if config.is_null()
            || out_runtime.is_null()
            || !config.is_aligned()
            || !out_runtime.is_aligned()
        {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        // SAFETY: Pointers were checked non-null/aligned and the ABI contract
        // requires them to be valid for this call.
        let config = unsafe { ptr::read(config) };
        if config.abi_version != ABI_VERSION || config.flags != 0 {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Ok(core) = Runtime::new(RuntimeConfig {
            max_inflight_requests: config.max_inflight_requests,
            max_completion_bytes: config.max_completion_bytes,
        }) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let owner_id = NEXT_RESOURCE_OWNER.fetch_add(1, Ordering::Relaxed);
        let Some(resource_owner) = ResourceOwner::new(owner_id) else {
            return RtStatus::new(RT_INTERNAL);
        };
        let maximum = config.max_inflight_requests as usize;
        let notifier = Arc::new(Mutex::new(None));
        let runtime = Box::new(RtRuntime {
            core: Mutex::new(core),
            notifier,
            filesystem: Mutex::new(None),
            filesystem_handles: Mutex::new(FilesystemHandles {
                next_id: 1,
                maximum,
                handles: HashMap::new(),
            }),
            filesystem_cancellations: Mutex::new(HashMap::new()),
            resource_owner,
            max_inflight_requests: maximum,
            max_completion_bytes: config.max_completion_bytes as usize,
        });
        // SAFETY: `out_runtime` is valid for writes by the ABI contract and is
        // written only after every fallible validation succeeds.
        unsafe { ptr::write(out_runtime, Box::into_raw(runtime)) };
        RtStatus::new(RT_OK)
    })
}

/// Registers a thread-safe notification callback. Passing `None` unregisters it.
/// The callback may run on any submitting/completing thread and must only
/// schedule a drain; it must not call JSC or reenter this runtime.
///
/// # Safety
///
/// `runtime` must be a live pointer returned by `rt_runtime_new`. `user_data`
/// must remain valid for callback use until unregistered or runtime free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_set_notifier(
    runtime: *mut RtRuntime,
    callback: Option<NotifierCallback>,
    user_data: *mut c_void,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Ok(mut current) = runtime.notifier.lock() else {
            return RtStatus::new(RT_INTERNAL);
        };
        *current = callback.map(|callback| Notifier {
            callback,
            user_data: user_data as usize,
        });
        RtStatus::new(RT_OK)
    })
}

/// Starts the runtime without running a caller-thread event loop.
///
/// # Safety
///
/// `runtime` must be a live pointer returned by `rt_runtime_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_start(runtime: *mut RtRuntime) -> RtStatus {
    with_core(runtime, Runtime::start)
}

/// Submits owned request bytes after copying them during this call.
///
/// # Safety
///
/// `runtime` must be live. When `payload_len` is non-zero, `payload` must point
/// to that many readable bytes. The pointer is not retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_submit(
    runtime: *mut RtRuntime,
    request_id: u64,
    op_id: u32,
    payload: *const u8,
    payload_len: usize,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(payload) = input_bytes(payload, payload_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let result = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| core.submit(request_id, op_id, payload));
        map_result(result)
    })
}

/// Submits a one-shot timer at an absolute monotonic tick.
///
/// # Safety
///
/// `runtime` must be a live pointer returned by `rt_runtime_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_submit_timer(
    runtime: *mut RtRuntime,
    request_id: u64,
    op_id: u32,
    deadline_tick: u64,
) -> RtStatus {
    with_core(runtime, |core| {
        core.submit_timer(request_id, op_id, deadline_tick)
    })
}

/// Moves a bounded number of elapsed timers to the normal completion queue.
/// The caller supplies monotonic ticks from the same origin used at submission.
///
/// # Safety
///
/// `runtime` and `out_fired` must be live and properly aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fire_due_timers(
    runtime: *mut RtRuntime,
    now_tick: u64,
    maximum: u32,
    out_fired: *mut u32,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if maximum == 0 || out_fired.is_null() || !out_fired.is_aligned() {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let result = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| core.fire_due_timers(now_tick, maximum as usize));
        match result {
            Ok(fired) => {
                let Ok(fired) = u32::try_from(fired) else {
                    return RtStatus::new(RT_INTERNAL);
                };
                // SAFETY: The pointer passed non-null/alignment validation and
                // the ABI contract guarantees one writable `u32`.
                unsafe { ptr::write(out_fired, fired) };
                if fired > 0 {
                    notify(runtime);
                }
                RtStatus::new(RT_OK)
            }
            Err(error) => map_result(Err(error)),
        }
    })
}

/// Reads the next timer deadline without blocking or consuming it.
///
/// # Safety
///
/// Both output pointers must be live and properly aligned. `out_present` is
/// written as 0 or 1; no C/Rust `bool` crosses the ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_next_timer_deadline(
    runtime: *mut RtRuntime,
    out_present: *mut u32,
    out_deadline_tick: *mut u64,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if out_present.is_null()
            || !out_present.is_aligned()
            || out_deadline_tick.is_null()
            || !out_deadline_tick.is_aligned()
        {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Ok(core) = runtime.core.lock() else {
            return RtStatus::new(RT_INTERNAL);
        };
        let deadline = core.next_timer_deadline();
        // SAFETY: Both output pointers passed validation and the ABI contract
        // guarantees they are writable for this call.
        unsafe {
            ptr::write(out_present, u32::from(deadline.is_some()));
            ptr::write(out_deadline_tick, deadline.unwrap_or(0));
        }
        RtStatus::new(RT_OK)
    })
}

/// Opens an explicitly approved directory and returns only a runtime-local
/// opaque object identity. This call may block and must run on a host worker.
///
/// # Safety
///
/// Runtime and output must be live/aligned. Locator pointer requirements match
/// `rt_runtime_submit`; locator bytes must be UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_open_approved_directory(
    runtime: *mut RtRuntime,
    locator: *const u8,
    locator_len: usize,
    rights: u64,
    out_object_id: *mut u64,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if out_object_id.is_null() || !out_object_id.is_aligned() {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Some(locator) = input_bytes(locator, locator_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Ok(locator) = std::str::from_utf8(locator) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if runtime
            .core
            .lock()
            .map_or(true, |core| core.state() != RuntimeState::Running)
        {
            return RtStatus::new(RT_RUNTIME_SHUTTING_DOWN);
        }
        let Ok(rights) = FilesystemRights::from_bits(rights) else {
            return operation_failure(ErrorCode::PermissionDenied);
        };
        let Ok(approved) = ApprovedDirectory::from_trusted_approval(PathBuf::from(locator), rights)
        else {
            return operation_failure(ErrorCode::InvalidArgument);
        };
        let Ok(filesystem) = filesystem(runtime) else {
            return RtStatus::new(RT_INTERNAL);
        };
        let Some(filesystem) = filesystem.as_ref() else {
            return RtStatus::new(RT_INTERNAL);
        };
        let handle = match filesystem.open_approved_directory(&approved) {
            Ok(handle) => handle,
            Err(error) => return operation_failure(error_code_for_filesystem(error)),
        };
        let object_id = match register_filesystem_handle(runtime, filesystem, handle) {
            Ok(object_id) => object_id,
            Err(status) => return status,
        };
        // SAFETY: The output pointer passed validation and is writable by the ABI contract.
        unsafe { ptr::write(out_object_id, object_id) };
        RtStatus::new(RT_OK)
    })
}

/// Returns the fixed-width identity of an opened directory for trusted
/// permission persistence code. This function is not installed into JS.
///
/// # Safety
///
/// Runtime must be live and `out_identity` writable for exactly 16 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_directory_identity(
    runtime: *mut RtRuntime,
    object_id: u64,
    out_identity: *mut u8,
    capacity: usize,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if out_identity.is_null() || capacity != 16 {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let handle = runtime
            .filesystem_handles
            .lock()
            .ok()
            .and_then(|handles| handles.handles.get(&object_id).copied());
        let identity = match (runtime.filesystem.lock(), handle) {
            (Ok(filesystem), Some(handle)) => filesystem
                .as_ref()
                .ok_or(FilesystemError::InvalidResource)
                .and_then(|filesystem| filesystem.directory_identity(handle)),
            _ => Err(FilesystemError::InvalidResource),
        };
        let identity = match identity {
            Ok(identity) => identity.authenticated_bytes(),
            Err(error) => return operation_failure(error_code_for_filesystem(error)),
        };
        // SAFETY: Caller provides a writable 16-byte output and the fixed local
        // array cannot overlap that caller-owned storage.
        unsafe { ptr::copy_nonoverlapping(identity.as_ptr(), out_identity, identity.len()) };
        RtStatus::new(RT_OK)
    })
}

/// Reopens an imported directory only when its actual identity matches the
/// authenticated envelope, returning a fresh runtime-local opaque object ID.
///
/// # Safety
///
/// Runtime/output must be live. Locator and identity pointers must be readable
/// for their declared lengths; identity length must be exactly 16.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_open_imported_directory(
    runtime: *mut RtRuntime,
    locator: *const u8,
    locator_len: usize,
    rights: u64,
    identity: *const u8,
    identity_len: usize,
    out_object_id: *mut u64,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if out_object_id.is_null() || !out_object_id.is_aligned() {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Some(locator) = input_bytes(locator, locator_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Ok(locator) = std::str::from_utf8(locator) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(identity) = input_bytes(identity, identity_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Ok(identity) = DirectoryIdentity::from_authenticated_bytes(identity) else {
            return operation_failure(ErrorCode::PermissionDenied);
        };
        if runtime
            .core
            .lock()
            .map_or(true, |core| core.state() != RuntimeState::Running)
        {
            return RtStatus::new(RT_RUNTIME_SHUTTING_DOWN);
        }
        let Ok(rights) = FilesystemRights::from_bits(rights) else {
            return operation_failure(ErrorCode::PermissionDenied);
        };
        let Ok(approved) = ApprovedDirectory::from_trusted_approval(PathBuf::from(locator), rights)
        else {
            return operation_failure(ErrorCode::InvalidArgument);
        };
        let Ok(filesystem) = filesystem(runtime) else {
            return RtStatus::new(RT_INTERNAL);
        };
        let Some(filesystem) = filesystem.as_ref() else {
            return RtStatus::new(RT_INTERNAL);
        };
        let handle = match filesystem.open_imported_directory(&approved, identity) {
            Ok(handle) => handle,
            Err(error) => return operation_failure(error_code_for_filesystem(error)),
        };
        let object_id = match register_filesystem_handle(runtime, filesystem, handle) {
            Ok(object_id) => object_id,
            Err(status) => return status,
        };
        // SAFETY: The output pointer passed validation and is writable by the ABI contract.
        unsafe { ptr::write(out_object_id, object_id) };
        RtStatus::new(RT_OK)
    })
}

/// Opens a typed regular-file resource beneath an authorized directory. This
/// blocking call must run on a host/filesystem worker, never the JS executor.
///
/// # Safety
///
/// Runtime/output must be live; relative path bytes must be readable UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_open_relative(
    runtime: *mut RtRuntime,
    directory_object_id: u64,
    path: *const u8,
    path_len: usize,
    rights: u64,
    create: u32,
    truncate: u32,
    out_file_object_id: *mut u64,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if out_file_object_id.is_null()
            || !out_file_object_id.is_aligned()
            || create > 1
            || truncate > 1
        {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Some(path) = validated_relative_path(path, path_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Ok(rights) = FilesystemRights::from_bits(rights) else {
            return operation_failure(ErrorCode::PermissionDenied);
        };
        let handle = runtime
            .filesystem_handles
            .lock()
            .ok()
            .and_then(|handles| handles.handles.get(&directory_object_id).copied());
        let Ok(filesystem) = filesystem(runtime) else {
            return RtStatus::new(RT_INTERNAL);
        };
        let Some(filesystem) = filesystem.as_ref() else {
            return RtStatus::new(RT_INTERNAL);
        };
        let file_handle = match handle
            .ok_or(FilesystemError::InvalidResource)
            .and_then(|handle| {
                filesystem.open_relative(handle, &path, rights, create != 0, truncate != 0)
            }) {
            Ok(handle) => handle,
            Err(error) => return operation_failure(error_code_for_filesystem(error)),
        };
        let object_id = match register_filesystem_handle(runtime, filesystem, file_handle) {
            Ok(object_id) => object_id,
            Err(status) => return status,
        };
        // SAFETY: Validated output is writable for one fixed-width identity.
        unsafe { ptr::write(out_file_object_id, object_id) };
        RtStatus::new(RT_OK)
    })
}

/// Submits a bounded read on an already-open typed file resource.
///
/// # Safety
///
/// Runtime must be live; all other arguments are fixed-width values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_read(
    runtime: *mut RtRuntime,
    request_id: u64,
    file_object_id: u64,
    max_bytes: u32,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        submit_filesystem_job(
            runtime,
            request_id,
            file_object_id,
            runtime_ops::builtin::FS_READ_FILE_ID,
            |filesystem, handle| {
                filesystem.submit_file_read(request_id, handle, max_bytes as usize)
            },
        )
    })
}

/// Submits a bounded write on an already-open typed file resource.
///
/// # Safety
///
/// Runtime must be live and data must be readable for `data_len` bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_write(
    runtime: *mut RtRuntime,
    request_id: u64,
    file_object_id: u64,
    data: *const u8,
    data_len: usize,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(data) = input_bytes(data, data_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if data.len() > runtime.max_completion_bytes {
            return RtStatus::new(RT_QUOTA_EXCEEDED);
        }
        let data = data.to_vec();
        submit_filesystem_job(
            runtime,
            request_id,
            file_object_id,
            runtime_ops::builtin::FS_WRITE_FILE_ID,
            move |filesystem, handle| filesystem.submit_file_write(request_id, handle, data),
        )
    })
}

/// Submits metadata on an already-open typed file resource.
///
/// # Safety
///
/// Runtime must be live; all other arguments are fixed-width values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_stat(
    runtime: *mut RtRuntime,
    request_id: u64,
    file_object_id: u64,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        submit_filesystem_job(
            runtime,
            request_id,
            file_object_id,
            runtime_ops::builtin::FS_METADATA_ID,
            |filesystem, handle| filesystem.submit_file_metadata(request_id, handle),
        )
    })
}

/// Submits a bounded descriptor-relative read to the filesystem worker pool.
///
/// # Safety
///
/// Runtime must be live and path pointer requirements match `rt_runtime_submit`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_read_file(
    runtime: *mut RtRuntime,
    request_id: u64,
    object_id: u64,
    path: *const u8,
    path_len: usize,
    max_bytes: u32,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(path) = validated_relative_path(path, path_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        submit_filesystem_operation(
            runtime,
            request_id,
            object_id,
            runtime_ops::builtin::FS_READ_FILE_ID,
            FilesystemOperation::ReadFile {
                path,
                max_bytes: max_bytes as usize,
            },
        )
    })
}

/// Submits a bounded descriptor-relative write to the filesystem worker pool.
///
/// # Safety
///
/// Runtime must be live. Path/data pointers follow the usual borrowed-input contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_write_file(
    runtime: *mut RtRuntime,
    request_id: u64,
    object_id: u64,
    path: *const u8,
    path_len: usize,
    data: *const u8,
    data_len: usize,
    create: u32,
    truncate: u32,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if create > 1 || truncate > 1 {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Some(path) = validated_relative_path(path, path_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(data) = input_bytes(data, data_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if data.len() > runtime.max_completion_bytes {
            return RtStatus::new(RT_QUOTA_EXCEEDED);
        }
        submit_filesystem_operation(
            runtime,
            request_id,
            object_id,
            runtime_ops::builtin::FS_WRITE_FILE_ID,
            FilesystemOperation::WriteFile {
                path,
                data: data.to_vec(),
                create: create == 1,
                truncate: truncate == 1,
            },
        )
    })
}

/// Submits descriptor-relative metadata lookup.
///
/// # Safety
///
/// Runtime and path must satisfy the usual live borrowed-input contract.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_metadata(
    runtime: *mut RtRuntime,
    request_id: u64,
    object_id: u64,
    path: *const u8,
    path_len: usize,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(path) = validated_relative_path(path, path_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        submit_filesystem_operation(
            runtime,
            request_id,
            object_id,
            runtime_ops::builtin::FS_METADATA_ID,
            FilesystemOperation::Metadata { path },
        )
    })
}

/// Closes one opaque directory resource and queues an ordinary completion.
///
/// # Safety
///
/// Runtime must be a live pointer returned by `rt_runtime_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_fs_close(
    runtime: *mut RtRuntime,
    request_id: u64,
    object_id: u64,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let submit = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| {
                core.submit(request_id, runtime_ops::builtin::RESOURCE_CLOSE_ID, &[])
            });
        if submit.is_err() {
            return map_result(submit);
        }
        let handle = runtime
            .filesystem_handles
            .lock()
            .ok()
            .and_then(|handles| handles.handles.get(&object_id).copied());
        let result = match (runtime.filesystem.lock(), handle) {
            (Ok(filesystem), Some(handle)) => filesystem
                .as_ref()
                .ok_or(FilesystemError::InvalidResource)
                .and_then(|filesystem| filesystem.close(handle)),
            _ => Err(FilesystemError::InvalidResource),
        };
        if result.is_ok() {
            if let Ok(mut handles) = runtime.filesystem_handles.lock() {
                handles.handles.remove(&object_id);
            }
        }
        complete_filesystem_result(runtime, request_id, result.map(|()| Vec::new()));
        notify(runtime);
        RtStatus::new(RT_OK)
    })
}

/// Transfers at most `maximum` worker results into the core completion queue.
///
/// # Safety
///
/// Runtime and output count must be live and aligned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_pump_filesystem(
    runtime: *mut RtRuntime,
    maximum: u32,
    out_pumped: *mut u32,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if maximum == 0 || out_pumped.is_null() || !out_pumped.is_aligned() {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Ok(filesystem) = runtime.filesystem.lock() else {
            return RtStatus::new(RT_INTERNAL);
        };
        let Some(filesystem) = filesystem.as_ref() else {
            // SAFETY: Output pointer passed validation and is writable.
            unsafe { ptr::write(out_pumped, 0) };
            return RtStatus::new(RT_OK);
        };
        let mut pumped = 0;
        while pumped < maximum {
            let completion = match filesystem.next_completion() {
                Ok(Some(completion)) => completion,
                Ok(None) => break,
                Err(_) => return RtStatus::new(RT_INTERNAL),
            };
            complete_worker_result(runtime, completion);
            pumped += 1;
        }
        // SAFETY: Output pointer passed validation and is writable.
        unsafe { ptr::write(out_pumped, pumped) };
        RtStatus::new(RT_OK)
    })
}

/// Completes a host operation and notifies after releasing internal locks.
///
/// # Safety
///
/// Pointer requirements are the same as `rt_runtime_submit`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_complete_host_op(
    runtime: *mut RtRuntime,
    request_id: u64,
    status: u32,
    payload: *const u8,
    payload_len: usize,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let Some(payload) = input_bytes(payload, payload_len) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let result = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| core.complete_host_op(request_id, status, payload));
        if result.is_ok() {
            notify(runtime);
        }
        map_result(result)
    })
}

/// Cancels a pending operation exactly once.
///
/// # Safety
///
/// `runtime` must be a live pointer returned by `rt_runtime_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_cancel(runtime: *mut RtRuntime, request_id: u64) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let result = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| core.cancel(request_id));
        if result.is_ok() {
            if let Ok(mut cancellations) = runtime.filesystem_cancellations.lock() {
                if let Some(cancellation) = cancellations.remove(&request_id) {
                    cancellation.cancel();
                }
            }
            notify(runtime);
        }
        map_result(result)
    })
}

/// Begins shutdown without blocking the caller and queues terminal completions.
///
/// # Safety
///
/// `runtime` must be a live pointer returned by `rt_runtime_new`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_shutdown(runtime: *mut RtRuntime) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let result = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| core.shutdown());
        if result.is_ok() {
            if let Ok(mut cancellations) = runtime.filesystem_cancellations.lock() {
                for cancellation in cancellations.values() {
                    cancellation.cancel();
                }
                cancellations.clear();
            }
            notify(runtime);
        }
        map_result(result)
    })
}

/// Copies the next completion into a caller-owned buffer without blocking.
/// A small buffer reports the required size without consuming the completion.
///
/// # Safety
///
/// `runtime` and `required_or_written` must be live and aligned. If capacity is
/// sufficient and non-zero, `out` must point to that many writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_next_completion(
    runtime: *mut RtRuntime,
    out: *mut u8,
    capacity: usize,
    required_or_written: *mut usize,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        if required_or_written.is_null() || !required_or_written.is_aligned() {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Ok(mut core) = runtime.core.lock() else {
            return RtStatus::new(RT_INTERNAL);
        };
        let Some(required) = core.next_completion_len() else {
            // SAFETY: The output-size pointer passed validation and the ABI
            // contract guarantees it is writable for this call.
            unsafe { ptr::write(required_or_written, 0) };
            return RtStatus::new(RT_EMPTY);
        };
        // SAFETY: The output-size pointer passed validation and the ABI contract
        // guarantees it is writable for this call.
        unsafe { ptr::write(required_or_written, required) };
        if capacity < required {
            return RtStatus::new(RT_BUFFER_TOO_SMALL);
        }
        if required > 0 && out.is_null() {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        }
        let Some(completion) = core.next_completion() else {
            return RtStatus::new(RT_INTERNAL);
        };
        // SAFETY: The caller guarantees `out` is writable for `capacity` bytes;
        // capacity was checked against the exact completion length. The Rust
        // source allocation cannot overlap caller-owned output.
        unsafe { ptr::copy_nonoverlapping(completion.as_ptr(), out, completion.len()) };
        RtStatus::new(RT_OK)
    })
}

/// Frees a runtime allocated by Rust. Null is ignored. Concurrent calls and
/// double free are forbidden by the ABI contract.
///
/// # Safety
///
/// `runtime` must be null or the unique live pointer returned by
/// `rt_runtime_new`, and must not be used again.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rt_runtime_free(runtime: *mut RtRuntime) {
    let _ = catch_unwind(AssertUnwindSafe(|| {
        if !runtime.is_null() && runtime.is_aligned() {
            // SAFETY: The ABI contract gives this function unique ownership of
            // the live allocation and forbids concurrent use or double free.
            drop(unsafe { Box::from_raw(runtime) });
        }
    }));
}

fn filesystem(runtime: &RtRuntime) -> Result<MutexGuard<'_, Option<Filesystem>>, FilesystemError> {
    let mut filesystem = runtime
        .filesystem
        .lock()
        .map_err(|_| FilesystemError::Internal)?;
    if filesystem.is_none() {
        let notifier: Arc<dyn CompletionNotifier> =
            Arc::new(NotifierProxy(runtime.notifier.clone()));
        *filesystem = Some(Filesystem::new(
            runtime.resource_owner,
            runtime.max_inflight_requests,
            2,
            runtime.max_inflight_requests,
            runtime.max_inflight_requests,
            runtime.max_completion_bytes,
            notifier,
        )?);
    }
    Ok(filesystem)
}

fn register_filesystem_handle(
    runtime: &RtRuntime,
    filesystem: &Filesystem,
    handle: ResourceHandle,
) -> Result<u64, RtStatus> {
    let Ok(mut handles) = runtime.filesystem_handles.lock() else {
        let _ = filesystem.close(handle);
        return Err(RtStatus::new(RT_INTERNAL));
    };
    if handles.handles.len() >= handles.maximum {
        let _ = filesystem.close(handle);
        return Err(operation_failure(ErrorCode::QuotaExceeded));
    }
    let object_id = handles.next_id;
    let Some(next_id) = object_id.checked_add(1) else {
        let _ = filesystem.close(handle);
        return Err(RtStatus::new(RT_INTERNAL));
    };
    handles.next_id = next_id;
    handles.handles.insert(object_id, handle);
    Ok(object_id)
}

fn validated_relative_path(path: *const u8, path_len: usize) -> Option<RelativePath> {
    let bytes = input_bytes(path, path_len)?;
    let text = std::str::from_utf8(bytes).ok()?;
    RelativePath::parse(text).ok()
}

fn submit_filesystem_operation(
    runtime: &RtRuntime,
    request_id: u64,
    object_id: u64,
    op_id: u32,
    operation: FilesystemOperation,
) -> RtStatus {
    submit_filesystem_job(
        runtime,
        request_id,
        object_id,
        op_id,
        move |filesystem, handle| filesystem.submit(request_id, handle, operation),
    )
}

fn submit_filesystem_job(
    runtime: &RtRuntime,
    request_id: u64,
    object_id: u64,
    op_id: u32,
    submit_job: impl FnOnce(&Filesystem, ResourceHandle) -> Result<CancellationToken, FilesystemError>,
) -> RtStatus {
    let submit = runtime
        .core
        .lock()
        .map_err(|_| RuntimeError::InvalidState)
        .and_then(|mut core| core.submit(request_id, op_id, &[]));
    if submit.is_err() {
        return map_result(submit);
    }
    let handle = runtime
        .filesystem_handles
        .lock()
        .ok()
        .and_then(|handles| handles.handles.get(&object_id).copied());
    let result = match (runtime.filesystem.lock(), handle) {
        (Ok(filesystem), Some(handle)) => filesystem
            .as_ref()
            .ok_or(FilesystemError::InvalidResource)
            .and_then(|filesystem| submit_job(filesystem, handle)),
        _ => Err(FilesystemError::InvalidResource),
    };
    match result {
        Ok(cancellation) => {
            if let Ok(mut cancellations) = runtime.filesystem_cancellations.lock() {
                cancellations.insert(request_id, cancellation);
            } else {
                complete_filesystem_result(runtime, request_id, Err(FilesystemError::Internal));
                notify(runtime);
            }
        }
        Err(error) => {
            complete_filesystem_result(runtime, request_id, Err(error));
            notify(runtime);
        }
    }
    RtStatus::new(RT_OK)
}

fn complete_worker_result(runtime: &RtRuntime, completion: WorkerCompletion) {
    if let Ok(mut cancellations) = runtime.filesystem_cancellations.lock() {
        cancellations.remove(&completion.request_id);
    }
    let result = completion.result.map_err(error_code_for_worker);
    let completion = match result {
        Ok(payload) => runtime.core.lock().ok().and_then(|mut core| {
            core.complete_host_op(completion.request_id, 0, &payload)
                .ok()
        }),
        Err(code) => runtime.core.lock().ok().and_then(|mut core| {
            core.complete_host_op(completion.request_id, code as u32, &[])
                .ok()
        }),
    };
    let _ = completion;
}

fn complete_filesystem_result(
    runtime: &RtRuntime,
    request_id: u64,
    result: Result<Vec<u8>, FilesystemError>,
) {
    if let Ok(mut core) = runtime.core.lock() {
        match result {
            Ok(payload) => {
                let _ = core.complete_host_op(request_id, 0, &payload);
            }
            Err(error) => {
                let _ =
                    core.complete_host_op(request_id, error_code_for_filesystem(error) as u32, &[]);
            }
        }
    }
}

const fn error_code_for_worker(error: WorkerError) -> ErrorCode {
    match error {
        WorkerError::Cancelled => ErrorCode::Cancelled,
        WorkerError::InvalidArgument => ErrorCode::InvalidArgument,
        WorkerError::InvalidResource => ErrorCode::InvalidResource,
        WorkerError::PermissionDenied => ErrorCode::PermissionDenied,
        WorkerError::ResourceClosed => ErrorCode::ResourceClosed,
        WorkerError::NotSupported => ErrorCode::NotSupported,
        WorkerError::QuotaExceeded | WorkerError::QueueFull => ErrorCode::QuotaExceeded,
        WorkerError::Io => ErrorCode::Io,
        WorkerError::Protocol => ErrorCode::Protocol,
        WorkerError::ShuttingDown => ErrorCode::RuntimeShuttingDown,
        WorkerError::WorkerFailed => ErrorCode::Internal,
    }
}

const fn error_code_for_filesystem(error: FilesystemError) -> ErrorCode {
    match error {
        FilesystemError::InvalidPath => ErrorCode::InvalidArgument,
        FilesystemError::InvalidResource => ErrorCode::InvalidResource,
        FilesystemError::PermissionDenied
        | FilesystemError::SymlinkDenied
        | FilesystemError::InvalidIdentity
        | FilesystemError::IdentityMismatch => ErrorCode::PermissionDenied,
        FilesystemError::ResourceClosed => ErrorCode::ResourceClosed,
        FilesystemError::WrongKind | FilesystemError::NotSupported => ErrorCode::NotSupported,
        FilesystemError::QuotaExceeded => ErrorCode::QuotaExceeded,
        FilesystemError::Cancelled => ErrorCode::Cancelled,
        FilesystemError::Protocol => ErrorCode::Protocol,
        FilesystemError::ShuttingDown => ErrorCode::RuntimeShuttingDown,
        FilesystemError::NotFound | FilesystemError::AlreadyExists | FilesystemError::Io => {
            ErrorCode::Io
        }
        FilesystemError::Internal => ErrorCode::Internal,
    }
}

const fn operation_failure(code: ErrorCode) -> RtStatus {
    RtStatus {
        status: RT_OPERATION_FAILED,
        detail: code as u32,
    }
}

fn boundary(operation: impl FnOnce() -> RtStatus) -> RtStatus {
    catch_unwind(AssertUnwindSafe(operation)).unwrap_or(RtStatus::new(RT_INTERNAL))
}

fn runtime_ref<'a>(runtime: *mut RtRuntime) -> Option<&'a RtRuntime> {
    if runtime.is_null() || !runtime.is_aligned() {
        return None;
    }
    // SAFETY: All callers document that a non-null aligned pointer must refer
    // to a live `RtRuntime` allocation for the duration of the call.
    Some(unsafe { &*runtime })
}

fn input_bytes<'a>(payload: *const u8, payload_len: usize) -> Option<&'a [u8]> {
    if payload_len == 0 {
        return Some(&[]);
    }
    if payload.is_null() {
        return None;
    }
    // SAFETY: FFI callers guarantee `payload_len` readable bytes. No reference
    // survives the exported function call and `u8` has alignment one.
    Some(unsafe { std::slice::from_raw_parts(payload, payload_len) })
}

fn with_core(
    runtime: *mut RtRuntime,
    operation: impl FnOnce(&mut Runtime) -> Result<(), RuntimeError>,
) -> RtStatus {
    boundary(|| {
        let Some(runtime) = runtime_ref(runtime) else {
            return RtStatus::new(RT_INVALID_ARGUMENT);
        };
        let result = runtime
            .core
            .lock()
            .map_err(|_| RuntimeError::InvalidState)
            .and_then(|mut core| operation(&mut core));
        map_result(result)
    })
}

fn notify(runtime: &RtRuntime) {
    let notifier = runtime.notifier.lock().ok().and_then(|guard| *guard);
    if let Some(notifier) = notifier {
        (notifier.callback)(notifier.user_data as *mut c_void);
    }
}

fn map_result(result: Result<(), RuntimeError>) -> RtStatus {
    let status = match result {
        Ok(()) => RT_OK,
        Err(RuntimeError::InvalidConfig | RuntimeError::InvalidArgument) => RT_INVALID_ARGUMENT,
        Err(RuntimeError::InvalidState) => RT_INVALID_STATE,
        Err(RuntimeError::DuplicateRequest) => RT_DUPLICATE_REQUEST,
        Err(RuntimeError::UnknownRequest) => RT_UNKNOWN_REQUEST,
        Err(RuntimeError::PayloadTooLarge | RuntimeError::QuotaExceeded) => RT_QUOTA_EXCEEDED,
        Err(RuntimeError::RuntimeShuttingDown) => RT_RUNTIME_SHUTTING_DOWN,
        Err(RuntimeError::Timer(error)) => match error {
            runtime_event_loop::timer::TimerError::InvalidRequest
            | runtime_event_loop::timer::TimerError::InvalidBudget => RT_INVALID_ARGUMENT,
            runtime_event_loop::timer::TimerError::DuplicateRequest => RT_DUPLICATE_REQUEST,
            runtime_event_loop::timer::TimerError::InvalidTimer => RT_UNKNOWN_REQUEST,
            runtime_event_loop::timer::TimerError::QuotaExceeded => RT_QUOTA_EXCEEDED,
            runtime_event_loop::timer::TimerError::ShuttingDown => RT_RUNTIME_SHUTTING_DOWN,
            runtime_event_loop::timer::TimerError::IdentifierExhausted => RT_INTERNAL,
        },
        Err(RuntimeError::Codec(_)) => RT_INTERNAL,
    };
    RtStatus::new(status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use runtime_core::codec::decode_completion;
    use runtime_filesystem::FilesystemRights;
    use std::fs;
    use std::sync::atomic::{AtomicPtr, AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    extern "C" fn count_notification(user_data: *mut c_void) {
        if user_data.is_null() {
            return;
        }
        // SAFETY: The test registers a pointer to a live `AtomicUsize` and
        // unregisters/frees the runtime before that value is dropped.
        let counter = unsafe { &*user_data.cast::<AtomicUsize>() };
        counter.fetch_add(1, Ordering::SeqCst);
    }

    struct ReentrantDrain {
        runtime: AtomicPtr<RtRuntime>,
        drained: AtomicUsize,
    }

    extern "C" fn drain_notification(user_data: *mut c_void) {
        if user_data.is_null() {
            return;
        }
        // SAFETY: The test-owned callback state and runtime stay live until
        // notifier unregistration and runtime free after callback completion.
        let state = unsafe { &*user_data.cast::<ReentrantDrain>() };
        let runtime = state.runtime.load(Ordering::Acquire);
        let mut required = 0;
        // SAFETY: Runtime and stack output are live; null output requests size.
        if unsafe {
            rt_runtime_next_completion(runtime, ptr::null_mut(), 0, &raw mut required).status
        } != RT_BUFFER_TOO_SMALL
        {
            return;
        }
        let mut output = vec![0_u8; required];
        // SAFETY: The output buffer has the exact reported writable capacity.
        if unsafe {
            rt_runtime_next_completion(
                runtime,
                output.as_mut_ptr(),
                output.len(),
                &raw mut required,
            )
            .status
        } == RT_OK
        {
            state.drained.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn new_runtime() -> *mut RtRuntime {
        let config = RtConfig {
            abi_version: ABI_VERSION,
            flags: 0,
            max_inflight_requests: 2,
            max_completion_bytes: 8,
        };
        let mut runtime = ptr::null_mut();
        // SAFETY: Test pointers are aligned and valid for this call.
        let status = unsafe { rt_runtime_new(&raw const config, &raw mut runtime) };
        assert_eq!(status.status, RT_OK);
        runtime
    }

    fn reopen_with_authenticated_identity(
        runtime: *mut RtRuntime,
        object_id: u64,
        locator: &[u8],
        rights: u64,
    ) {
        let mut identity = [0_u8; 16];
        // SAFETY: Test runtime is live and trusted output has the exact required capacity.
        assert_eq!(
            unsafe {
                rt_runtime_fs_directory_identity(
                    runtime,
                    object_id,
                    identity.as_mut_ptr(),
                    identity.len(),
                )
            }
            .status,
            RT_OK
        );
        let mut imported_object_id = 0;
        // SAFETY: Authenticated identity, locator, and output remain live for the call.
        assert_eq!(
            unsafe {
                rt_runtime_fs_open_imported_directory(
                    runtime,
                    locator.as_ptr(),
                    locator.len(),
                    rights,
                    identity.as_ptr(),
                    identity.len(),
                    &raw mut imported_object_id,
                )
            }
            .status,
            RT_OK
        );
        assert_ne!(imported_object_id, object_id);
        let mut wrong_identity = identity;
        wrong_identity[15] ^= 1;
        let mut denied_object_id = 0;
        // SAFETY: Inputs remain live; modified authenticated metadata is intentionally rejected.
        assert_eq!(
            unsafe {
                rt_runtime_fs_open_imported_directory(
                    runtime,
                    locator.as_ptr(),
                    locator.len(),
                    rights,
                    wrong_identity.as_ptr(),
                    wrong_identity.len(),
                    &raw mut denied_object_id,
                )
            }
            .status,
            RT_OPERATION_FAILED
        );
        assert_eq!(denied_object_id, 0);
    }

    #[test]
    // Verifies null and malformed configuration fail without panic or allocation.
    fn new_rejects_invalid_pointers_and_version() {
        // SAFETY: Null is intentionally supplied to verify boundary validation.
        let status = unsafe { rt_runtime_new(ptr::null(), ptr::null_mut()) };
        assert_eq!(status.status, RT_INVALID_ARGUMENT);
        let config = RtConfig {
            abi_version: ABI_VERSION + 1,
            flags: 0,
            max_inflight_requests: 1,
            max_completion_bytes: 1,
        };
        let mut runtime = ptr::null_mut();
        // SAFETY: Test pointers are aligned and valid for this call.
        let status = unsafe { rt_runtime_new(&raw const config, &raw mut runtime) };
        assert_eq!(status.status, RT_INVALID_ARGUMENT);
        assert!(runtime.is_null());
    }

    #[test]
    // Verifies caller-buffer completion transfer and notifier behavior end to end.
    fn submit_complete_and_drain_are_allocator_safe() {
        let runtime = new_runtime();
        let notifications = AtomicUsize::new(0);
        // SAFETY: Runtime and user-data pointers remain live through the test.
        assert_eq!(
            unsafe {
                rt_runtime_set_notifier(
                    runtime,
                    Some(count_notification),
                    (&raw const notifications).cast_mut().cast(),
                )
            }
            .status,
            RT_OK
        );
        // SAFETY: Runtime is live and empty payload permits a null pointer.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        assert_eq!(
            unsafe { rt_runtime_submit(runtime, 1, 1, ptr::null(), 0) }.status,
            RT_OK
        );
        assert_eq!(
            unsafe { rt_runtime_complete_host_op(runtime, 1, 0, ptr::null(), 0) }.status,
            RT_OK
        );
        assert_eq!(notifications.load(Ordering::SeqCst), 1);
        let mut required = 0;
        // SAFETY: Runtime and size output are live; null output asks for size.
        assert_eq!(
            unsafe { rt_runtime_next_completion(runtime, ptr::null_mut(), 0, &raw mut required) }
                .status,
            RT_BUFFER_TOO_SMALL
        );
        let mut output = vec![0; required];
        // SAFETY: Output allocation has the reported writable capacity.
        assert_eq!(
            unsafe {
                rt_runtime_next_completion(
                    runtime,
                    output.as_mut_ptr(),
                    output.len(),
                    &raw mut required,
                )
            }
            .status,
            RT_OK
        );
        assert_eq!(
            decode_completion(&output, 8).expect("decoded").request_id,
            1
        );
        // SAFETY: Runtime pointer is uniquely owned and not used afterward.
        unsafe { rt_runtime_free(runtime) };
    }

    #[test]
    // Verifies non-zero input length with null data is rejected safely.
    fn submit_rejects_null_nonempty_payload() {
        let runtime = new_runtime();
        // SAFETY: Runtime is live; malformed payload is intentional.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        assert_eq!(
            unsafe { rt_runtime_submit(runtime, 1, 1, ptr::null(), 1) }.status,
            RT_INVALID_ARGUMENT
        );
        // SAFETY: Runtime pointer is uniquely owned and not used afterward.
        unsafe { rt_runtime_free(runtime) };
    }

    #[test]
    // Verifies misaligned structs and overflow-sized null buffers fail at the boundary.
    #[allow(clippy::cast_ptr_alignment)]
    fn malformed_alignment_and_length_are_rejected() {
        let storage = [0_u32; 8];
        let misaligned = (&raw const storage)
            .cast::<u8>()
            .wrapping_add(1)
            .cast::<RtConfig>();
        let mut runtime = ptr::null_mut();
        // SAFETY: Misalignment is intentional and detected before dereference.
        assert_eq!(
            unsafe { rt_runtime_new(misaligned, &raw mut runtime) }.status,
            RT_INVALID_ARGUMENT
        );
        let runtime = new_runtime();
        // SAFETY: Runtime is live; null with maximum length is intentionally malformed.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        assert_eq!(
            unsafe { rt_runtime_submit(runtime, 1, 1, ptr::null(), usize::MAX) }.status,
            RT_INVALID_ARGUMENT
        );
        // SAFETY: Runtime pointer is uniquely owned and not used afterward.
        unsafe { rt_runtime_free(runtime) };
    }

    #[test]
    // Verifies timer deadline inspection and bounded firing use the ordinary notifier/completion ABI.
    fn timer_submission_fires_through_completion_queue() {
        let runtime = new_runtime();
        let notifications = AtomicUsize::new(0);
        // SAFETY: Runtime and notification state remain live for the test.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        assert_eq!(
            unsafe {
                rt_runtime_set_notifier(
                    runtime,
                    Some(count_notification),
                    (&raw const notifications).cast_mut().cast(),
                )
            }
            .status,
            RT_OK
        );
        // SAFETY: Runtime is live and scalar arguments need no borrowed memory.
        assert_eq!(
            unsafe { rt_runtime_submit_timer(runtime, 1, 2, 10) }.status,
            RT_OK
        );
        let mut present = 0;
        let mut deadline = 0;
        // SAFETY: Output scalars are aligned and live.
        assert_eq!(
            unsafe { rt_runtime_next_timer_deadline(runtime, &raw mut present, &raw mut deadline) }
                .status,
            RT_OK
        );
        assert_eq!((present, deadline), (1, 10));
        let mut fired = 0;
        // SAFETY: Output scalar and runtime are live.
        assert_eq!(
            unsafe { rt_runtime_fire_due_timers(runtime, 9, 1, &raw mut fired) }.status,
            RT_OK
        );
        assert_eq!(fired, 0);
        assert_eq!(notifications.load(Ordering::SeqCst), 0);
        assert_eq!(
            unsafe { rt_runtime_fire_due_timers(runtime, 10, 1, &raw mut fired) }.status,
            RT_OK
        );
        assert_eq!(fired, 1);
        assert_eq!(notifications.load(Ordering::SeqCst), 1);
        // SAFETY: Runtime pointer is uniquely owned and not used afterward.
        unsafe { rt_runtime_free(runtime) };
    }

    #[test]
    // Verifies real concurrent cancel/complete calls have exactly one winner and one completion.
    fn concurrent_cancel_complete_race_settles_once() {
        let runtime = new_runtime();
        // SAFETY: Runtime is live throughout both joined calls.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        assert_eq!(
            unsafe { rt_runtime_submit(runtime, 1, 1, ptr::null(), 0) }.status,
            RT_OK
        );
        let barrier = Arc::new(Barrier::new(3));
        let runtime_address = runtime as usize;
        let complete_barrier = barrier.clone();
        let complete = std::thread::spawn(move || {
            complete_barrier.wait();
            // SAFETY: The pointer is reconstructed only while the owner waits
            // for this thread and no free can occur.
            unsafe {
                rt_runtime_complete_host_op(runtime_address as *mut RtRuntime, 1, 0, ptr::null(), 0)
                    .status
            }
        });
        let cancel_barrier = barrier.clone();
        let cancel = std::thread::spawn(move || {
            cancel_barrier.wait();
            // SAFETY: The pointer lifetime is bounded by the joined owner.
            unsafe { rt_runtime_cancel(runtime_address as *mut RtRuntime, 1).status }
        });
        barrier.wait();
        let mut statuses = [
            complete.join().expect("complete"),
            cancel.join().expect("cancel"),
        ];
        statuses.sort_unstable();
        assert_eq!(statuses, [RT_OK, RT_UNKNOWN_REQUEST]);
        let mut required = 0;
        // SAFETY: Runtime and size output remain live.
        assert_eq!(
            unsafe { rt_runtime_next_completion(runtime, ptr::null_mut(), 0, &raw mut required) }
                .status,
            RT_BUFFER_TOO_SMALL
        );
        // SAFETY: Runtime pointer is uniquely owned and not used afterward.
        unsafe { rt_runtime_free(runtime) };
    }

    #[test]
    // Verifies a notifier may reenter completion drain because no core lock is held.
    fn notifier_can_reenter_nonblocking_drain() {
        let runtime = new_runtime();
        let state = ReentrantDrain {
            runtime: AtomicPtr::new(runtime),
            drained: AtomicUsize::new(0),
        };
        // SAFETY: Runtime and callback state remain live and aligned.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        assert_eq!(
            unsafe {
                rt_runtime_set_notifier(
                    runtime,
                    Some(drain_notification),
                    (&raw const state).cast_mut().cast(),
                )
            }
            .status,
            RT_OK
        );
        assert_eq!(
            unsafe { rt_runtime_submit(runtime, 1, 1, ptr::null(), 0) }.status,
            RT_OK
        );
        assert_eq!(
            unsafe { rt_runtime_complete_host_op(runtime, 1, 0, ptr::null(), 0) }.status,
            RT_OK
        );
        assert_eq!(state.drained.load(Ordering::SeqCst), 1);
        // SAFETY: Unregistration precedes unique runtime free.
        unsafe {
            let _ = rt_runtime_set_notifier(runtime, None, ptr::null_mut());
            rt_runtime_free(runtime);
        }
    }

    #[test]
    // Verifies approved directory open, worker read, pump, completion, and close through the C ABI.
    fn filesystem_vertical_slice_uses_opaque_identity() {
        let runtime = new_runtime();
        // SAFETY: Runtime remains live through the test.
        assert_eq!(unsafe { rt_runtime_start(runtime) }.status, RT_OK);
        let directory = std::env::temp_dir().join(format!(
            "zintl-ffi-fs-{}-{}",
            std::process::id(),
            NEXT_RESOURCE_OWNER.load(Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("directory");
        let directory = fs::canonicalize(directory).expect("canonical directory");
        fs::write(directory.join("data.bin"), [1_u8, 2, 3]).expect("fixture");
        let locator = directory.to_str().expect("UTF-8 locator").as_bytes();
        let rights = FilesystemRights::READ
            .union(FilesystemRights::METADATA)
            .bits();
        let mut object_id = 0;
        // SAFETY: Locator and object output are valid for this call.
        assert_eq!(
            unsafe {
                rt_runtime_fs_open_approved_directory(
                    runtime,
                    locator.as_ptr(),
                    locator.len(),
                    rights,
                    &raw mut object_id,
                )
            }
            .status,
            RT_OK
        );
        assert_ne!(object_id, 0);
        reopen_with_authenticated_identity(runtime, object_id, locator, rights);
        let path = b"data.bin";
        // SAFETY: Runtime and path bytes remain live for the call.
        assert_eq!(
            unsafe { rt_runtime_fs_read_file(runtime, 1, object_id, path.as_ptr(), path.len(), 4) }
                .status,
            RT_OK
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut pumped = 0;
        while pumped == 0 && Instant::now() < deadline {
            // SAFETY: Runtime and output count remain live.
            assert_eq!(
                unsafe { rt_runtime_pump_filesystem(runtime, 1, &raw mut pumped) }.status,
                RT_OK
            );
            if pumped == 0 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        assert_eq!(pumped, 1);
        let mut required = 0;
        // SAFETY: Runtime and output size remain live.
        assert_eq!(
            unsafe { rt_runtime_next_completion(runtime, ptr::null_mut(), 0, &raw mut required) }
                .status,
            RT_BUFFER_TOO_SMALL
        );
        let mut encoded = vec![0_u8; required];
        // SAFETY: Completion output has the exact reported size.
        assert_eq!(
            unsafe {
                rt_runtime_next_completion(
                    runtime,
                    encoded.as_mut_ptr(),
                    encoded.len(),
                    &raw mut required,
                )
            }
            .status,
            RT_OK
        );
        assert_eq!(
            decode_completion(&encoded, 8)
                .expect("read completion")
                .payload,
            vec![1, 2, 3]
        );
        // SAFETY: Runtime is live and the object identity is virtual, not a descriptor.
        assert_eq!(
            unsafe { rt_runtime_fs_close(runtime, 2, object_id) }.status,
            RT_OK
        );
        // SAFETY: Runtime pointer is uniquely owned and not used afterward.
        unsafe { rt_runtime_free(runtime) };
        fs::remove_dir_all(&directory).expect("cleanup");
    }
}
