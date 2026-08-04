# Rust embedding API

`runtime-embed` is the safe public host surface. `RuntimeBuilder` registers
finite limits, a one-shot permission callback and custom operations before
`build`. `start` freezes configuration without starting a caller-thread loop.

```rust
use runtime_embed::{PermissionDecision, PermissionRequest, PermissionResponder, RuntimeBuilder};

let runtime = RuntimeBuilder::new()
    .permission_callback(|request: PermissionRequest, response: PermissionResponder| {
        let decision = if trusted_policy_allows(&request) {
            PermissionDecision::Allow {
                rights: request.requested_rights,
                quota: 64 * 1024,
            }
        } else {
            PermissionDecision::Deny
        };
        let _ = response.respond(decision);
    })
    .build()?;
runtime.start()?;
# fn trusted_policy_allows(_: &PermissionRequest) -> bool { false }
# Ok::<(), runtime_embed::RuntimeError>(())
```

No callback means no authority. The callback receives descriptive request data
and a consuming responder, but no capability, resource ID, path authority or OS
handle. Rust retains the requested scope and rejects unknown rights, broader
rights, zero quota, duplicate/late responses, timeout and shutdown races.

`Directory` and `FileResource` are typed opaque capabilities. They expose no
`AsFd`, `AsRawFd` or platform operation. Filesystem work uses descriptor-relative
walking on bounded workers and denies symlinks and traversal.

`EngineSession` owns a `Box<dyn JavaScriptEngineBackend>` and maps engine object
IDs to Rust resources. `drive(DriveBudget)` is non-blocking and bounded by item
and byte count. Embedders schedule drive turns from their executor. The CLI may
wait between turns; UI/main executors must not use blocking `RuntimeTask::wait`.

The crate-level Rustdoc is authoritative for public ownership, error, callback,
threading, cancellation and shutdown contracts.
