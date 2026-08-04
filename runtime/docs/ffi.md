# JavaScriptCore private C ABI

The only C ABI is the private Rust-to-Swift engine boundary declared by
`include/runtime_jsc_ffi.h`. It uses fixed-width integers, explicit byte
pointer/length pairs, caller-owned output buffers and an opaque engine pointer.
No Rust runtime, resource, descriptor or engine value crosses it.

Events use canonical `ZJE1` envelopes with big-endian fixed-width fields.
Unknown version/kind, invalid UTF-8, truncation, noncanonical flags and trailing
bytes fail closed. `next_event` is non-blocking; a small output buffer reports
the required size without consuming the event.

Swift copies retained input bytes. Rust owns completion buffers. The notifier
is wake-only and may be coalesced. Rust does not free the opaque engine until
Swift has synchronously released all JSC values on its serial queue.
