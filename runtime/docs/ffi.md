# C ABI and adapter contract

The public ABI is versioned and uses only fixed-width integers, explicit-layout
C structs, `const uint8_t * + size_t`, caller-provided output buffers, and an
opaque `rt_runtime_t *`. It never exposes Rust/Swift/JSC/C++ layout, `bool`,
trait objects, variable-layout enums, OS handles, or engine values.

Inputs are checked for ABI and schema version, nullability, alignment where
required, length overflow, configured size limits, duplicate request IDs,
runtime state, and valid UTF-8 where the schema requires text. Bytes retained
after a call are copied into Rust-owned storage. Unknown values fail closed.

Every exported Rust function added in M3 is a narrow `catch_unwind` boundary and
returns a stable `rt_status_t`. Caller-provided completion buffers avoid
cross-allocator ownership; a too-small buffer reports the required size without
partial consumption. `next_completion` is always non-blocking and returns an
empty status when no item exists.

Timer submission uses absolute monotonic ticks supplied from a single caller
clock origin. Deadline inspection is non-blocking, and `fire_due_timers` has a
mandatory item budget. It writes elapsed timers into the ordinary completion
queue and invokes the same notifier after releasing the core lock. Presence is
represented as `uint32_t` 0/1 rather than an ABI-unstable language boolean.

The notifier callback is thread-safe and conveys only that draining is needed.
It may coalesce notifications and cannot call JSC. Swift schedules bounded drain
work on the embedder's serial JS executor. Host-op and permission callbacks have
explicit registration lifetime, executor, cancellation, timeout, shutdown, and
late/double-completion contracts before callable ABI is exposed.

The trusted persistence adapter can read a fixed 16-byte directory identity and
reopen a directory using an authenticated identity. Those calls remain on the
Swift host surface, are not captured by the JS bootstrap, and return only a new
opaque runtime-local object ID. Identity mismatch fails before resource-table
insertion.

Concurrent `rt_runtime_free` with another call is outside the ABI contract and
will be documented in the generated header. Shutdown itself is idempotent or
returns a defined state error and never blocks the main thread.
