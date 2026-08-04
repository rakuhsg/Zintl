# ADR-0007: libc syscall bindings

Status: Accepted

`reactor-kqueue` depends on the pure-Rust `libc` crate with default features. It
provides audited platform declarations and constants for `kqueue`, `kevent`,
`fcntl`, `dup`, and `pipe` without a build script, native library, network API,
process launcher, or dynamic-loading facility. Hand-maintaining these ABI layouts
would create greater memory-safety and portability risk.

The dependency is confined to `reactor-kqueue`; core crates do not depend on it.
Unsafe calls remain in thin wrappers with local `SAFETY` arguments, checked
return values, initialized buffers, RAII descriptor ownership, and conformance
tests. No libc constants or descriptor numbers cross reactor-api, FFI, or JS.
