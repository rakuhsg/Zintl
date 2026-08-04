# JavaScriptCore adapter

`RuntimeJSC` confines the `JSContext`, all `JSValue` instances, native callback
blocks, Promise callbacks, and host-object decoration to the serial executor
provided by the embedder. The executor must not be the main queue; the adapter
checks its execution precondition whenever it enters JavaScriptCore.

The bootstrap captures the minimal native callbacks and then deletes their
global properties. Untrusted code sees only the frozen `Zintl` API. Capability
and resource objects carry identity in a JavaScriptCore private slot allocated
by `RuntimeJSCShim`; ordinary JavaScript properties contain no runtime ID,
resource ID, pointer, descriptor, or other authority. Every receiver is checked
for the expected kind and runtime before use.

## Operation and completion flow

1. `Zintl.invoke` validates its public arguments and creates a Promise.
2. Swift copies the input bytes and submits a request identity to the Rust C ABI.
3. The explicit host dispatcher executes away from the JavaScript executor and
   reports success or a structured failure through `rt_runtime_complete_host_op`.
4. The Rust core transactionally moves the request into its bounded completion
   queue and invokes the thread-safe notifier without holding a core lock.
5. The notifier only coalesces and schedules work. It never enters JSC.
6. The JS executor drains at most 64 completions or 1 MiB per turn, decodes the
   versioned envelope, settles each Promise once, and performs one microtask
   checkpoint for the batch. Remaining work is rescheduled as another turn.

The Swift package links the Rust `runtime-ffi` static library. Embedders and CI
must run `cargo build --manifest-path runtime/Cargo.toml -p runtime-ffi` before
building the Swift package.

## Shutdown

Shutdown rejects new work, marks and cancels pending Rust requests, cancels and
awaits cooperative host tasks, shuts down the Rust runtime, drains terminal
completions, and rejects anything still pending. Only then does it unregister
the notifier, release all JSC references on their executor, and free the opaque
runtime. A queued notifier turn observes the cleared runtime and becomes a
no-op, so it cannot access a freed C handle.
