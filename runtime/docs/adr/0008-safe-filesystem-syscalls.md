# ADR-0008: Safe filesystem syscall bindings

Status: Accepted

M5 uses `rustix` for safe owned-descriptor wrappers around `openat`, `statat`,
`mkdirat`, `unlinkat`, `renameat`, directory iteration, `dup`, and `fstat`.
`unicode-normalization` enforces NFC operation paths. This keeps unsafe code out
of the policy/resource/filesystem crates while retaining descriptor-relative
macOS primitives. Both dependencies are version-locked by `Cargo.lock`, built
with the workspace MSRV, and exercised by traversal, symlink, rights, resource,
quota, and operation tests.
