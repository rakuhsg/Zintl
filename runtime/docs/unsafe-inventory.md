# Unsafe inventory

Unsafe code is denied in every crate except `runtime-ffi` and
`reactor-kqueue`. The workspace strict lint still reviews all other warnings.

## runtime-ffi

- Read/write of validated C pointers during a call. Null and alignment are
  checked first; the caller contract supplies validity and non-aliasing.
- Temporary input slices. Non-zero lengths require non-null readable storage;
  bytes are not retained and core copies data it stores.
- Caller-buffer completion copies. Capacity is checked against the exact queued
  size before a non-overlapping copy.
- `Box::from_raw` in `rt_runtime_free`. The ABI explicitly requires unique live
  ownership and forbids concurrent calls or double free.
- Test-only callback pointer access and FFI calls, each paired with a local
  `SAFETY` argument.

Every exported function is a `catch_unwind` boundary. FFI tests cover null,
misalignment, unsupported version, null/non-zero length, maximum length,
caller-buffer sizing, notification, allocation/free ownership, and normal
request lifecycle. Timer tests also cover aligned scalar outputs, deadline
presence, bounded firing, notification, and completion-queue delivery.
Concurrency tests race cancel against completion through the real ABI and prove
that a notifier can reenter non-blocking drain, demonstrating callbacks run
after the core mutex is released.

Directory persistence adds only fixed 16-byte caller-buffer identity transfer
and the existing validated borrowed-input pattern. Reopen compares identity
before resource insertion. The fuzz target calls the ABI only with live aligned
Rust storage and uniquely frees each successfully created runtime.

## reactor-kqueue

- `kqueue`, `kevent`, `fcntl`, `dup`, and test-only `pipe` syscall calls. Inputs
  are initialized and aligned; return values and interruption are checked.
- `OwnedFd::from_raw_fd` immediately after successful descriptor creation or
  duplication. Each descriptor is wrapped exactly once and closed by RAII.
- `kevent` output uses a fully initialized fixed array; only the returned count
  is read. `udata` stores a numeric slot/generation token, never a Rust pointer.
- Test-only `fcntl(F_GETFD)` probes former descriptor numbers after drop to prove
  closure; the values are not exposed in API output or snapshots.

Reactor tests cover register/reregister/deregister, read readiness, wake,
deadline, EOF/hangup, stale registration, deregister/close sequencing,
background-thread polling, and descriptor cleanup.
