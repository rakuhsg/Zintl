#[cfg(target_os = "macos")]
fn main() {
    use std::process::Command;

    // Cargo does not propagate executable linker arguments from library
    // dependencies, so the UI test host carries Swift's runtime paths.
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

    println!("cargo:rustc-link-arg=-rpath");
    println!("cargo:rustc-link-arg={swift_runtime}");
    println!("cargo:rustc-link-arg=-rpath");
    println!("cargo:rustc-link-arg=/usr/lib/swift");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
