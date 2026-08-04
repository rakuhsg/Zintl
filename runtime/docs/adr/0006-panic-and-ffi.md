# ADR-0006: Panic containment and FFI

Status: Accepted

All callable Rust C exports are protected by narrow `catch_unwind` wrappers and
return stable internal-error status on panic. No unwinding crosses C ABI. The
core and safe crates forbid unsafe code; M3 inventories the localized unsafe in
the FFI and kqueue syscall wrappers, with a `SAFETY` rationale and negative tests
for each boundary.

