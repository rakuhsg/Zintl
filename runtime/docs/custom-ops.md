# Custom operations and callback lifecycle

Registration is configuration-only. `OpRegistryBuilder` validates a stable ID,
name/version, schema version, input/output bounds, permission kind, execution
class, and timeout. `freeze()` consumes the builder; the running registry has no
mutation API. Custom operations cannot use `zintl.builtin.*`, and neither a
name/version nor stable ID may collide.

Dispatch looks up the immutable descriptor, validates request metadata and input
size, invokes the permission authorizer, and only then yields `PreparedOp`.
Handlers receive owned validated bytes and metadata—not JSC values, OS handles,
or mutable resource-table access. Synchronous and asynchronous output uses the
same descriptor limit.

`CallbackTracker` registers a bounded request before external code runs. Complete,
cancel, timeout, and shutdown race to one terminal state. Late and duplicate
results cannot replace it. Host work is submitted through `HostOpExecutor`; the
executor must enqueue without blocking the JS executor. Permission callbacks
follow the same lifetime and exactly-once rules and run without Rust locks.

Swift's `EmbeddedRuntime.registerOp` is configuration-only and concise. Missing
permission resolution denies execution. The registered async closure is run by
an explicit `HostOpExecutor`, not by MainActor or the JSC serial executor.

The descriptor timeout covers permission resolution and host execution together.
The adapter races that work against a monotonic deadline and accepts only the
first result. Timeout or cancellation cancels the losing task but does not await
a non-cooperative callback, so third-party async code cannot hold runtime
shutdown open. A late callback result is discarded before it can reach the Rust
completion queue. Swift cannot forcibly terminate arbitrary async code; a
callback that ignores cancellation may retain only its own captured state until
it returns, never JSC values, the adapter, or a pending Promise.

Trusted allow decisions are still validated: scope must match the requested
opaque scope, requested rights must be present, and quota must be non-zero.
Malformed opaque decisions fail closed.
