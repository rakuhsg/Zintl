# Adding engine and reactor backends

## JavaScript engines

A V8 or other engine adapter implements `JavaScriptEngineAdapter` and the same
versioned C ABI/op envelopes. It must own all engine-native contexts, values,
callbacks, private host-object slots, and Promise roots on one host-supplied
serial executor. Engine values and native pointers cannot enter Rust or public
errors. Run `EngineConformanceHarness` plus the shared security scenarios before
adding backend-specific tests. Do not add an ambient filesystem, network,
process, dynamic-loading, environment, or module-loader global.

The adapter must preserve bounded completion draining, explicit microtask
checkpoints, structured stable errors, private receiver validation, exactly-once
settlement, and context-before-runtime shutdown ordering. Core, permission,
resource, op, and filesystem crates must not change to mention the engine.

## Readiness reactors

An epoll or IOCP readiness backend implements `reactor_api::Reactor` using
portable `Interest`, `ReactorEvent`, and `EventFlags`; OS constants and handles
remain in its platform crate. Registrations are opaque and generational. Polling
runs only on a dedicated reactor thread, handles interruption internally, obeys
deadlines, sanitizes backend errors, coalesces wakeups, and bounds event drains.
Attach a real platform test source and run `run_readable_conformance`, then add
EOF/error, stale token, reregister/deregister, wake, close race, and cleanup
tests.

io_uring and IOCP completion operations must use a separate completion-driver
interface rather than pretending completion is readiness. Define ownership,
buffer pinning, cancellation, late completion, and shutdown semantics before
adding that interface. Core must depend only on the portable contract.
