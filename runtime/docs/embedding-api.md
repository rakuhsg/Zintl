# Embedding lifecycle and limits

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
