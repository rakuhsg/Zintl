use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=../../swift/Package.swift");
    println!("cargo:rerun-if-changed=../../swift/Sources/RuntimeJSCFFI");
    println!("cargo:rerun-if-changed=../../swift/Sources/RuntimeJSCShim");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let package = manifest_dir.join("../../swift");
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("out dir"));
    let scratch = out.join("swift-build");
    let module_cache = out.join("clang-module-cache");
    fs::create_dir_all(&module_cache).expect("create module cache");
    let profile = if env::var("PROFILE").as_deref() == Ok("release") {
        "release"
    } else {
        "debug"
    };
    let status = Command::new("swift")
        .args([
            "build",
            "--package-path",
            package.to_str().expect("utf8 package path"),
            "--scratch-path",
            scratch.to_str().expect("utf8 scratch path"),
            "--product",
            "RuntimeJSCFFI",
            "-c",
            profile,
        ])
        .env("CLANG_MODULE_CACHE_PATH", &module_cache)
        .env("SWIFTPM_MODULECACHE_OVERRIDE", &module_cache)
        .status()
        .expect("run swift build");
    assert!(status.success(), "failed to build RuntimeJSCFFI");
    let archive = find_archive(&scratch).expect("RuntimeJSCFFI static archive");
    println!(
        "cargo:rustc-link-search=native={}",
        archive.parent().expect("archive parent").display()
    );
    println!("cargo:rustc-link-lib=static=RuntimeJSCFFI");
    println!("cargo:rustc-link-lib=framework=JavaScriptCore");
    println!("cargo:rustc-link-lib=framework=Foundation");
    let target_info = Command::new("swiftc")
        .arg("-print-target-info")
        .output()
        .expect("swift target info");
    assert!(target_info.status.success(), "swiftc target info failed");
    let json = String::from_utf8(target_info.stdout).expect("utf8 target info");
    let mut in_paths = false;
    for line in json.lines() {
        if line.contains("\"runtimeLibraryPaths\"") {
            in_paths = true;
            continue;
        }
        if in_paths && line.contains(']') {
            break;
        }
        if in_paths {
            let path = line.trim().trim_end_matches(',').trim_matches('"');
            if Path::new(path).is_absolute() {
                println!("cargo:rustc-link-search=native={path}");
            }
        }
    }
}

fn find_archive(root: &Path) -> Option<PathBuf> {
    for entry in fs::read_dir(root).ok()? {
        let path = entry.ok()?.path();
        if path.is_dir() {
            if let Some(found) = find_archive(&path) {
                return Some(found);
            }
        } else if path
            .file_name()
            .is_some_and(|name| name == "libRuntimeJSCFFI.a")
        {
            return Some(path);
        }
    }
    None
}
