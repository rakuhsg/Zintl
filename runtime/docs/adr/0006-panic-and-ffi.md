# ADR-0006: Panic containment and FFI

Status: Accepted

The private Rust-to-Swift engine boundary returns stable status codes; Swift
errors and Rust panics do not cross the C ABI. Core and safe crates forbid
unsafe code. Unsafe is confined to the JSC adapter and kqueue syscall wrappers,
with a `SAFETY` rationale and negative tests for each boundary.
