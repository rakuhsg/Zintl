//! Compatibility build-script support for applications using `zintl-desktop`.

/// Configures the final application binary for `zintl-desktop`.
///
/// This is retained as a no-op so existing application build scripts continue
/// to compile. The Rust AppKit backend needs no executable linker arguments.
pub fn configure() {}
