# Security model

## Trust boundaries

JavaScript and all decoded request bytes are untrusted. VFS configuration and
application-owned authorities are trusted inputs to policy. OS resources stay
in Rust-owned objects.

## Application-owned authority

The application decides whether and how to persist approvals. The runtime asks
the selected VFS authority for every access and retains no approval state. An
authority panic is treated as denial. Unknown VFS names, operations, resource
kinds, schema versions, states, and unsupported platforms fail closed.

## Isolation and lifetime

Resource handles are runtime-scoped and generational. Lookup validates runtime,
slot, generation, state, kind, and rights. Close and shutdown reject new work;
slot reuse increments generation. VFS roots and opened files never become
JavaScript objects. Cross-runtime, stale, or closed resources are rejected.

No raw descriptor, native pointer, JSC value, internal capability ID, secret,
canonical path, or permission blob is placed in JS-visible properties, errors,
logs, audit events, payload snapshots, or UI.

## Filesystem policy

JavaScript filesystem access is rooted in host-registered virtual filesystems.
Scripts can resolve only `<vfs>://<relative-path>` URLs; absolute OS paths are
rejected and VFS source paths never cross the engine boundary. Each mount may carry an application-owned `Authority`
that decides every normalized path request. The runtime neither caches nor
persists those decisions. Empty components, NUL, absolute paths, `..`, platform
separators, excessive sizes, and normalization ambiguity are rejected before the
authority is consulted.

The filesystem backend uses descriptor-relative component walking and a deny-symlink policy; it does
not authorize through `canonicalize` plus prefix comparison. A registered VFS
root is opened once from the trusted host configuration. Operations use the
already-opened resource rather than re-resolving the configured path, preventing
authorization/use TOCTOU.
Every path component is checked before and after `openat` where a descriptor is
needed. Final-component remove and rename syscalls do not dereference a symlink;
the backend still rejects a symlink observed before the syscall. macOS does not
provide Linux `openat2`-style `RESOLVE_BENEATH`, so the implementation performs
explicit `openat` component walking and descriptor type checks instead.

## Failure safety and availability

All queues, payloads, buffers, resources, requests, timers, worker jobs, and
drains are bounded. Oversize input and exhaustion produce stable errors. Rust
panics cannot unwind through C ABI. Swift errors cannot cross C ABI. Diagnostics
sanitize errno, internal paths, payloads, tokens, and existence information.

JSC evaluations and retained host-object globals also have hard counts. The
implementation-owned builtin namespace fails closed before the custom host
dispatcher. Filesystem listing count is derived from its byte budget, writes
check cancellation between bounded chunks, and worker completions enforce their
own byte limit.

Cancellation, timeout, completion, close, and shutdown races use explicit state
transitions and exactly-once settlement. JSC is never called from a worker,
reactor or notifier.

## Threat review checklist

- ambient global or bypass op introduced
- capability attenuation can increase rights, scope, quota, or expiry
- forged/cross-runtime/stale handle accepted
- raw handle, pointer, secret, path, or payload appears at a boundary
- authority invoked under a lock or on an undeclared executor
- unbounded allocation, queue, loop, or worker creation
- path traversal, symlink escape, or authorization/use re-resolution
- duplicate or late completion settles more than once
- FFI null, length, version, ownership, panic, or shutdown state unvalidated
- network/process/environment/native-loading dependency or global introduced
