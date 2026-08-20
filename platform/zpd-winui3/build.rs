#[cfg(all(target_os = "windows", not(target_env = "msvc")))]
compile_error!("zpd-winui3 supports only the MSVC Windows targets");

#[cfg(target_os = "windows")]
fn main() {
    use std::env;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    let manifest_dir = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("manifest dir"));
    let support_dir = manifest_dir.join("../ZintlWinUI3Support");
    let project = support_dir.join("ZintlWinUI3Support.vcxproj");
    let configuration = if env::var("PROFILE").as_deref() == Ok("release") {
        "Release"
    } else {
        "Debug"
    };
    let platform = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86_64") => "x64",
        Ok("aarch64") => "ARM64",
        other => panic!("unsupported zpd-winui3 architecture: {other:?}"),
    };
    let msbuild = find_msbuild();

    println!(
        "cargo:rerun-if-changed={}",
        support_dir.join("include").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        support_dir.join("src").display()
    );
    println!("cargo:rerun-if-changed={}", project.display());

    let status = Command::new(msbuild)
        .arg(&project)
        .args(["/restore", "/m", "/nologo", "/verbosity:minimal"])
        .arg(format!("/p:Configuration={configuration}"))
        .arg(format!("/p:Platform={platform}"))
        .status()
        .expect("failed to run MSBuild");
    assert!(status.success(), "failed to build ZintlWinUI3Support");

    let output = support_dir.join("build").join(platform).join(configuration);
    let package_root = env::var_os("NUGET_PACKAGES").map_or_else(
        || {
            PathBuf::from(env::var_os("USERPROFILE").expect("USERPROFILE is not set"))
                .join(".nuget/packages")
        },
        PathBuf::from,
    );
    let foundation = package_root.join("microsoft.windowsappsdk.foundation/1.8.260203002");
    let native_arch = if platform == "x64" { "x64" } else { "arm64" };
    let runtime_arch = if platform == "x64" {
        "win-x64"
    } else {
        "win-arm64"
    };
    println!("cargo:rustc-link-search=native={}", output.display());
    println!(
        "cargo:rustc-link-search=native={}",
        foundation.join("lib/native").join(native_arch).display()
    );
    println!("cargo:rustc-link-lib=static=ZintlWinUI3Support");
    println!("cargo:rustc-link-lib=Microsoft.WindowsAppRuntime.Bootstrap");
    println!("cargo:rustc-link-lib=windowsapp");
    println!("cargo:rustc-link-lib=ole32");
    println!("cargo:rustc-link-lib=user32");

    let profile_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is not set"))
        .ancestors()
        .nth(3)
        .expect("unexpected Cargo OUT_DIR")
        .to_owned();
    let bootstrap_dll = foundation
        .join("runtimes")
        .join(runtime_arch)
        .join("native/Microsoft.WindowsAppRuntime.Bootstrap.dll");
    for directory in [&profile_dir, &profile_dir.join("deps")] {
        std::fs::create_dir_all(directory).expect("failed to create Cargo output directory");
        std::fs::copy(
            &bootstrap_dll,
            directory.join("Microsoft.WindowsAppRuntime.Bootstrap.dll"),
        )
        .expect("failed to copy the Windows App SDK bootstrap DLL");
    }

    fn find_msbuild() -> PathBuf {
        let installer = env::var_os("ProgramFiles(x86)")
            .map(PathBuf::from)
            .expect("ProgramFiles(x86) is not set")
            .join("Microsoft Visual Studio/Installer/vswhere.exe");
        let output = Command::new(installer)
            .args([
                "-latest",
                "-products",
                "*",
                "-requires",
                "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
                "-property",
                "installationPath",
            ])
            .output()
            .expect("failed to locate Visual Studio");
        assert!(output.status.success(), "vswhere failed");
        let root = String::from_utf8(output.stdout)
            .expect("Visual Studio path is not UTF-8")
            .trim()
            .to_owned();
        let path = Path::new(&root).join("MSBuild/Current/Bin/MSBuild.exe");
        assert!(
            path.is_file(),
            "MSBuild was not found at {}",
            path.display()
        );
        path
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {}
