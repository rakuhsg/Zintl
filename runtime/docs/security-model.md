# Security model

## Trust boundaries

JavaScript and all decoded request bytes are untrusted. The embedder,
permission resolver, permission codec, and scope-locator resolver are trusted
inputs to policy, but Rust validates that their decisions do not exceed the
request or imported authenticated envelope. OS resources stay in Rust-owned
objects.

## Deny-by-default authority

Unknown operations, permission kinds, resource kinds, schema versions, states,
and unsupported platforms fail closed. A capability can be minted only from a
validated embedder decision, authenticated import, or attenuation of an existing
capability. Derivation enforces subset scope and rights, non-increasing quota,
and non-extending expiry.

No callback means no authority. Custom operations use the same permission,
quota, cancellation, audit, and completion pipeline as built-ins. Development
and test configurations do not gain allow-all or root-filesystem shortcuts.

## Isolation and lifetime

Resource handles are runtime-scoped and generational. Lookup validates runtime,
slot, generation, state, kind, and rights. Close and shutdown reject new work;
slot reuse increments generation. JSC host objects use private branding and do
not expose the handle as ordinary writable properties. Cross-runtime, forged,
stale, closed, or receiver-spoofed objects are rejected.

No raw descriptor, native pointer, JSC value, internal capability ID, secret,
canonical path, or permission blob is placed in JS-visible properties, errors,
logs, audit events, payload snapshots, or UI.

## Permission persistence

Exported permission descriptions are versioned and authenticated by an
embedder-provided codec. Import validates exact issuer, audience, permission
kind, expiry, nonce replay, rights attenuation, quota attenuation, scope
identity, and reopened directory type before minting runtime-local authority.
Opaque locators are resolved only after codec authentication. Replay state is
bounded, and failed resolution/open does not consume the nonce or retain a
partial grant. See `permission-persistence.md` for the byte and callback
contracts.

## Filesystem policy

Filesystem authority is rooted in a directory capability. Except for the
untrusted directory locator used by a permission request, filesystem operations
accept only relative components. Empty components, NUL, absolute paths, `..`,
platform separators, excessive sizes, and normalization ambiguity are rejected.

The filesystem backend uses descriptor-relative component walking and a deny-symlink policy; it does
not authorize through `canonicalize` plus prefix comparison. The directory is
opened only after approval. Operations use the already-opened resource rather
than re-resolving the authorized path, preventing authorization/use TOCTOU.
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
reactor, notifier, or permission executor.

## Threat review checklist

- ambient global or bypass op introduced
- capability attenuation can increase rights, scope, quota, or expiry
- forged/cross-runtime/stale handle accepted
- raw handle, pointer, secret, path, or payload appears at a boundary
- callback invoked under a lock or on an undeclared executor
- unbounded allocation, queue, loop, or worker creation
- path traversal, symlink escape, or authorization/use re-resolution
- duplicate or late completion settles more than once
- FFI null, length, version, ownership, panic, or shutdown state unvalidated
- network/process/environment/native-loading dependency or global introduced
