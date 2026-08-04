# Embedding lifecycle and limits

## Rust API

`runtime-embed` is the engine-neutral public Rust embedding surface. A builder
owns configuration-time registration and produces a configured runtime; `start`
freezes that state without starting a resident loop or occupying the caller.

```rust
use runtime_embed::{
    HostOpContext, OpLimits, PermissionDecision, RuntimeBuilder,
};

let runtime = RuntimeBuilder::new()
    .permission_resolver(|request| PermissionDecision::Allow {
        scope: request.requested_scope,
        rights: request.requested_rights,
        quota: 64 * 1024,
    })
    .register_op(
        "dev.zintl.demo.reverse",
        1,
        OpLimits::new(64 * 1024, 64 * 1024, 30_000_000_000)?,
        "dev.zintl.permission.reverse",
        |_context: HostOpContext, mut bytes: Vec<u8>| {
            bytes.reverse();
            Ok(bytes)
        },
    )?
    .build()?;

runtime.start()?;
let output = runtime
    .invoke("dev.zintl.demo.reverse", 1, b"Zintl".to_vec(), vec![], 1)?
    .wait()?;
runtime.shutdown()?;
# Ok::<(), runtime_embed::RuntimeError>(())
```

Every submitted call returns `RuntimeTask<T>`, which implements `Future` and
also offers `wait` to trusted CLI/background threads. `cancel` settles the
public task once and sets the cooperative cancellation bit visible through
`HostOpContext`. Custom-op and permission deadlines settle outside the callback,
so a non-cooperative callback cannot retain the caller; bounded host workers and
queue capacity prevent unbounded execution growth.

`Directory` and `FileResource` are typed opaque Rust capabilities. They expose
no table identity or OS descriptor, automatically close on drop, fail after
shutdown, and run open/read/write/stat work away from the caller. Authenticated
directory export/import is configured with embedder-provided `PermissionCodec`
and `DirectoryScopeCodec` implementations; import attenuates rights/quota,
consumes replay state, resolves persistent scope, and rechecks directory identity
before returning a fresh `Directory`.

Engine crates implement `EngineAdapter` and receive only a weak `RuntimeHandle`.
Their concrete evaluation API stays engine-specific. `EngineLease` guarantees
adapter shutdown on explicit teardown or drop. The Rust JavaScript REPL is the
runnable end-to-end host using this API: it attaches Boa, keeps directory
capabilities in opaque Rust context data, and asks for every grant with an
explicit terminal `y/N` prompt. Terminal input remains on the REPL thread; the
runtime resolver consumes only a one-shot approval matching the displayed
operation, directory, and rights.

## Swift compatibility layer

`EmbeddedRuntimeConfiguration` validates permission timeout and bounded audit
capacity. `EmbeddedRuntime.build` creates a configured runtime; custom ops may
be registered only in that state. `startJavaScriptCore` freezes registration,
creates an adapter on the host-supplied serial executor, and tracks it as part
of the embedding lifecycle. Duplicate runtime IDs fail.

`EmbeddedRuntime.shutdown` is idempotent. It transitions `running` to
`shuttingDown`, concurrently asks each tracked adapter to cancel pending work,
drains defined shutdown completions, releases resources and JSC state on the JS
executor, then becomes `terminated`. New registration/start calls fail once
shutdown begins. The caller can await shutdown from a UI task; no main-thread
poll, resident loop, or busy wait is required. The compiled
`EmbeddingLifecycleExample` demonstrates build, non-main start, evaluation, and
shutdown.

Limits are mandatory at each layer: `OpLimits` bounds custom input/output and
timeout; permission callback timeout is configured globally; Rust configuration
bounds in-flight requests and completion bytes; adapter code bounds scripts,
evaluations, host objects, filesystem paths, operation bytes, drain work, and
timer duration; worker/reactor/resource tables have fixed capacities. Zero or
effectively unbounded public configuration is rejected. Exhaustion maps to a
stable quota error rather than allocation growth or process failure.

`auditSnapshot` returns a bounded ring ordered by sequence. `AuditEvent` can
represent only category, outcome, operation identity, and optional request ID.
It has no payload, locator, resource/capability ID, native handle, error text,
permission blob, or secret field. Oldest events are discarded at capacity.
