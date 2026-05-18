use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use deno_core::op2;
use deno_runtime::ops::bootstrap::SnapshotOptions;
use deno_runtime::snapshot::create_runtime_snapshot;

#[op2(fast)]
fn op_zintl_window_create() {}

deno_runtime::deno_core::extension!(
    zintl,
    ops = [op_zintl_window_create],
    esm_entry_point = "ext:zintl/window.ts",
    esm = ["ext:zintl/window.ts" = "../../libs/window.ts"],
);

fn main() {
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR must be set"));
    let snapshot_path = out_dir.join("ZINTL_DENO_SNAPSHOT.bin");

    create_runtime_snapshot(
        snapshot_path,
        SnapshotOptions::default(),
        vec![zintl::init()],
    );
    watch_libs(Path::new("../../libs"));
}

fn watch_libs(path: &Path) {
    println!("cargo:rerun-if-changed={}", path.display());

    let entries = fs::read_dir(path).expect("failed to read libs directory");
    for entry in entries {
        let path = entry.expect("failed to read libs directory entry").path();
        if path.is_dir() {
            watch_libs(&path);
        } else if path.extension().is_some_and(|extension| extension == "js") {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}
