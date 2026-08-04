fn main() {
    #[cfg(target_os = "macos")]
    {
        use std::process::Command;

        println!(
            "cargo:rerun-if-changed=../../platform/ZintlAppkitSupport/Sources/ZintlAppkitSupport/ZintlAppkitSupport.swift"
        );
        println!(
            "cargo:rerun-if-changed=../../platform/ZintlAppkitSupport/Sources/ZintlAppkitSupportTypes/include/zintlappkit.h"
        );

        // Get a path of Swift Runtime Library
        let path = String::from_utf8(
            Command::new("xcode-select")
                .args(["--print-path"])
                .output()
                .expect("failed to run xcode-select")
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        let status = Command::new("swift")
            .args(["build", "-c", "release"])
            .current_dir("../../platform/ZintlAppkitSupport")
            .status()
            .expect("failed to build ZintlAppkitSupport");

        if !status.success() {
            panic!("runtil: Failed to build XCode project");
        }

        // r-paths
        println!("cargo:rustc-link-arg=-rpath");
        println!(
            "cargo:rustc-link-arg={}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
            &path
        );
        println!("cargo:rustc-link-arg=-rpath");
        println!("cargo:rustc-link-arg={}", "/usr/lib/swift");

        println!(
            "cargo:rustc-link-search=../platform/ZintlAppkitSupport/.build/arm64-apple-macosx/release/"
        );
        println!(
            "cargo:rustc-link-search={}/Toolchains/XcodeDefault.xctoolchain/usr/lib/swift/macosx",
            &path
        );
        // ffi library
        println!("cargo:rustc-link-lib=static=ZintlAppkitSupport");
    }
}
