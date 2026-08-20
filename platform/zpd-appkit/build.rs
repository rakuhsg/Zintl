#[cfg(target_os = "macos")]
fn main() {
    use std::env;
    use std::path::PathBuf;

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let support_dir = manifest_dir.join("../ZintlAppkitSupport");
    emit_swift_support(&support_dir);
}

#[cfg(target_os = "macos")]
fn emit_swift_support(support_dir: &std::path::Path) {
    use std::process::Command;

    let module_cache = support_dir.join(".build/module-cache");
    std::fs::create_dir_all(&module_cache).expect("failed to create the Swift module cache");

    println!(
        "cargo:rerun-if-changed={}",
        support_dir.join("Sources").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        support_dir.join("Package.swift").display()
    );

    let status = Command::new("swift")
        .args(["build", "-c", "release", "--package-path"])
        .arg(support_dir)
        .env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .env("SWIFTPM_MODULECACHE_OVERRIDE", &module_cache)
        .status()
        .expect("failed to run swift build");
    assert!(status.success(), "failed to build ZintlAppkitSupport");

    let bin_path = Command::new("swift")
        .args([
            "build",
            "-c",
            "release",
            "--show-bin-path",
            "--package-path",
        ])
        .arg(support_dir)
        .env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .env("SWIFTPM_MODULECACHE_OVERRIDE", &module_cache)
        .output()
        .expect("failed to query the Swift build path");
    assert!(
        bin_path.status.success(),
        "failed to query the Swift build path"
    );
    let bin_path = String::from_utf8(bin_path.stdout)
        .expect("Swift build path must be UTF-8")
        .trim()
        .to_owned();

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

    println!("cargo:rustc-link-search=native={bin_path}");
    println!("cargo:rustc-link-search=native={swift_runtime}");
    println!("cargo:rustc-link-lib=static=ZintlAppkitSupport");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
