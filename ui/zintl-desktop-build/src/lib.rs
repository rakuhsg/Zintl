//! Build-script support for applications using `zintl-desktop`.

/// Configures the final application binary for `zintl-desktop`.
///
/// Call this function from the application's `build.rs`. On macOS, it adds
/// the Swift runtime search paths required by Zintl's AppKit support library.
/// On other platforms, it does nothing.
pub fn configure() {
    configure_platform();
}

#[cfg(target_os = "macos")]
fn configure_platform() {
    use std::process::Command;

    println!("cargo:rerun-if-env-changed=DEVELOPER_DIR");

    let developer_dir = Command::new("xcode-select")
        .arg("--print-path")
        .output()
        .expect("failed to run xcode-select");
    assert!(
        developer_dir.status.success(),
        "failed to query the Xcode developer path"
    );
    let developer_dir = String::from_utf8(developer_dir.stdout)
        .expect("Xcode developer path must be UTF-8")
        .trim()
        .to_owned();
    let swift_runtime =
        format!("{developer_dir}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx");

    emit_rpath(&swift_runtime);
    emit_rpath("/usr/lib/swift");
}

#[cfg(target_os = "macos")]
fn emit_rpath(path: &str) {
    println!("cargo:rustc-link-arg=-rpath");
    println!("cargo:rustc-link-arg={path}");
}

#[cfg(not(target_os = "macos"))]
fn configure_platform() {}
