# Rust embedding API

`runtime-embed` is the safe public host surface. `RuntimeBuilder` registers
finite limits, virtual filesystems and custom operations before `build`.
`start` freezes configuration without starting a caller-thread loop.

```rust
use runtime_embed::{
    Authority, AuthorizationRequest, AuthorizationResult, RuntimeBuilder,
    Source, VfsConfig,
};
use std::sync::Arc;

struct ProjectAuthority;

impl Authority for ProjectAuthority {
    fn authorization_requested(&self, request: &AuthorizationRequest<'_>) -> AuthorizationResult {
        if request.path == "project.json" {
            AuthorizationResult::Allow
        } else {
            AuthorizationResult::Deny
        }
    }
}

# let project_directory = std::path::PathBuf::from(".");
let runtime = RuntimeBuilder::new()
    .add_vfs(VfsConfig {
        name: "project".into(),
        source: Source::LoadDir { path: project_directory },
        authority: Some(Arc::new(ProjectAuthority)),
    })?
    .build()?;
runtime.start()?;
# Ok::<(), runtime_embed::RuntimeError>(())
```

`Authority` belongs to the application. It receives a normalized VFS-relative
path on every access, and the runtime does not retain or persist allow-list
state. An absent authority allows access to that explicitly registered mount.
JavaScript sees only URLs such as `project://project.json`; the host directory
never crosses the engine boundary.

VFS roots and opened files remain internal typed resources. They expose no
`AsFd`, `AsRawFd` or platform operation. Filesystem work uses
descriptor-relative walking on bounded workers and denies symlinks and
traversal.

`EngineSession` owns a `Box<dyn JavaScriptEngineBackend>` and routes typed VFS
requests to Rust. `drive(DriveBudget)` is non-blocking and bounded by item and
byte count. Embedders schedule drive turns from their executor. The CLI may wait
between turns; UI/main executors must not use blocking `RuntimeTask::wait`.

The crate-level Rustdoc is authoritative for public ownership, error,
authority, threading, cancellation and shutdown contracts.
