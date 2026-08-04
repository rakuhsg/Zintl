# Embedded runtime architecture

## Ownership

The application owns VFS authorization policy. Rust owns runtime state, policy
enforcement, capability/resource tables, timers, I/O workers, reactor
registrations and OS resources. Swift owns only the
JavaScriptCore context, values, Promise roots and private host-object slots.
Untrusted JavaScript receives opaque runtime-local object identities, never an
OS descriptor, native pointer or platform API.

```text
javascript-repl -> runtime-jsc -> RuntimeJSCFFI -> JavaScriptCore
                -> runtime-embed -> runtime-engine
                                 -> runtime-permission
                                 -> runtime-filesystem -> runtime-resource
                                 -> runtime-event-loop<NativeReactor>
                                      `-> reactor-kqueue [macOS]
```

Core, permission, resource and filesystem policy do not depend on a JavaScript
engine. `runtime-jsc` implements `JavaScriptEngineBackend`; a future V8 crate
implements the same trait and shared conformance tests.

## Event loops

`runtime-event-loop::EventLoop<R>` is generic over `reactor_api::Reactor`.
`NativeReactor` is a compile-time `cfg(target_os)` alias, not a trait object.
macOS selects `KqueueReactor`; unsupported targets fail closed until their
backend is added. Blocking readiness polling is confined to a dedicated driver
thread. Commands, registrations, events, completions and drains are bounded.

The kqueue backend owns descriptors with RAII, uses `EVFILT_USER` for wakeups,
maps native flags to portable semantics and discards stale generational tokens.
It is an internal, non-published package and is not re-exported by the embedding
API.

## Scheduling and lifecycle

Engine events and host completions cross a bounded versioned byte protocol. A
notifier conveys only that work may be available. It never enters JSC. The host
schedules finite `EngineSession::drive` turns; there is no public resident loop
or main-thread blocking poll.

Shutdown rejects new work, cancels host/timer/I/O requests, drains
terminal completions, deregisters the reactor, closes Rust resources, releases
JSC values on the Swift serial queue and finally frees the opaque engine.
