#[cfg(target_os = "macos")]
fn main() {
    use std::env;
    use std::path::PathBuf;
    use std::process::Command;

    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR must be set"));
    let support_dir = manifest_dir.join("../../ZintlAppkitSupport");

    println!(
        "cargo:rerun-if-changed={}",
        support_dir
            .join("Sources/ZintlAppkitSupport/ZintlAppkitSupport.swift")
            .display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        support_dir
            .join("Sources/ZintlAppkitSupportTypes/include/zintlappkit.h")
            .display()
    );

    let status = Command::new("swift")
        .args(["build", "-c", "release", "--package-path"])
        .arg(&support_dir)
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
        .arg(&support_dir)
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
    println!("cargo:rustc-link-arg=-rpath");
    println!("cargo:rustc-link-arg={swift_runtime}");
    println!("cargo:rustc-link-arg=-rpath");
    println!("cargo:rustc-link-arg=/usr/lib/swift");
}

#[cfg(not(target_os = "macos"))]
fn main() {}
