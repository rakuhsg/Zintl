# Adding backends

## JavaScript engines

A V8 backend implements `runtime_engine::JavaScriptEngineBackend`. It must own
all V8 values and Promise roots, emit the same typed events, validate private
receivers, perform explicit microtask checkpoints and pass the common engine
conformance and security tests. Runtime, permission, resource and filesystem
crates must not change to mention V8.

## Readiness reactors

A platform reactor implements `reactor_api::Reactor` and is selected through the
compile-time `NativeReactor` alias. Do not use `Box<dyn Reactor>`. Registrations
are opaque and generational; OS handles and native event constants remain in the
internal platform crate. A backend must pass readable/writable, EOF/error,
stale-token, reregister/deregister, wake, deadline, close-race and cleanup tests.
