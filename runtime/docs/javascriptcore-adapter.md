# JavaScriptCore backend

`runtime-jsc::JavaScriptCoreBackend` implements the Rust
`JavaScriptEngineBackend` trait. Its build script compiles and statically links
the Swift `RuntimeJSCFFI` product. Non-macOS builds provide a fail-closed
`Unsupported` backend.

Swift owns `JSContext`, `JSValue`, native blocks and Promise roots on a private
serial queue. JavaScript host calls enqueue typed requests for Rust. Rust owns
permission decisions and resources, then sends only bytes, stable failures or
opaque object IDs back to Swift.

Directory and file objects are allocated by `RuntimeJSCShim` with private
runtime ID, object ID and kind slots. Every method validates its receiver; no ID
or pointer is stored in a writable JavaScript property.

Evaluation settlement and host requests are bounded events. Rust drains them in
finite turns. Worker, reactor, notifier and permission callback threads never
enter JavaScriptCore.
