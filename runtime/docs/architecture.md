# Embedded runtime architecture

## Invariants

Untrusted JavaScript has no ambient OS authority. Every host operation follows
`decode → validate → runtime state → capability/resource lookup → authorize →
quota → execute → completion`. Data such as a path, string, option, or JSON
object is never authority. JavaScript and the C ABI receive virtual identities,
never an OS descriptor, socket, port, pointer, or engine value.

The Rust core is engine- and platform-neutral. Swift/JSC owns every JSC VM,
context, value, host object, and Promise callback. Rust owns permission state,
capabilities, resources, request state, completion queues, and OS resources.
The reactor owns readiness registrations.

## Dependency direction

```text
RuntimeEmbed -> RuntimeJSC -> CRuntimeFFI
                              |
javascript-repl -> Boa + runtime-embed
runtime-embed -> runtime-core -> runtime-ops -> runtime-permission
      |       -> runtime-filesystem -> runtime-resource
      `----------------------------> runtime-event-loop
runtime-ffi -> runtime-core -> runtime-ops -> runtime-permission
                    |              |        -> runtime-resource
                    |              `------> runtime-event-loop -> reactor-api
                    `-------------------------------------------> reactor-api
runtime-filesystem -----------------------------> runtime-resource
        `--------------------------------------> runtime-event-loop
reactor-kqueue -----------------------------------------------> reactor-api
```

`runtime-core` cannot depend on JSC, Swift, V8, kqueue types, or OS event flags.
`runtime-permission` cannot depend on an engine value. `runtime-resource` cannot
depend on a JS object. CI checks these forbidden edges.

Executable hosts remain outside `runtime/crates`. `runtime-embed` is a reusable
library, not an executable or engine backend. The single Rust CLI target at
`runtime/examples/javascript-repl` is an explicitly allowlisted host: it keeps
the pure-Rust Boa engine and terminal I/O at the outer edge and depends inward
on the public embedding contracts. Production runtime crates never depend on
the REPL, terminal I/O, or an engine.

## Threads and ownership

JSC runs only on the embedder-supplied serial executor. It is not implicitly the
main queue. A dedicated background reactor thread performs blocking readiness
polls. A separate bounded worker pool performs blocking filesystem work.
Workers and the reactor enqueue bounded completions and invoke only a thread-safe
notifier. The notifier schedules a bounded, non-blocking drain on the JS
executor; it never enters JSC itself.

Filesystem authority begins at an approved, opened directory descriptor.
`runtime-filesystem` walks every locator and operation component relative to an
already-open directory, refuses symlinks, duplicates the root descriptor before
queueing work, and never retains the resource-table lock during blocking I/O.

The embedding API exposes no `run`, `runForever`, or blocking `poll`. A drain has
both item and byte budgets and yields when either is reached. UI integration only
schedules tasks, so the runtime does not occupy the main thread.

Timers are one-shot entries in a bounded Rust queue keyed by absolute monotonic
ticks. Equal deadlines use insertion order. The adapter schedules only a
cooperative wake task; elapsed timers rejoin the ordinary Rust completion queue
and therefore use the same notifier, drain budget, settlement, and shutdown
rules as host operations.

## Lifecycle and races

Runtime state moves in one direction:

```text
Configured -> Running -> ShuttingDown -> Terminated
```

Shutdown rejects new submissions, atomically resolves cancel-vs-complete races,
settles each Promise once, closes resources, stops workers/reactor, and only then
permits context destruction. Embedder callbacks run without Rust internal locks
and on their declared executor. Late or duplicate completions are rejected and
never settle a Promise twice.

## Backend contracts

The engine boundary is stable C ABI plus versioned byte payloads. A future V8
adapter reuses op schemas, error codes, virtual-resource semantics, and engine
conformance tests. The reactor boundary uses semantic interest/event flags and
opaque generational registrations. Completion-oriented backends may add a
separate operation-driver interface rather than pretending to be readiness
reactors. See ADR-0004.
