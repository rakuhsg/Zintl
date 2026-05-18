use std::env;
use std::fs;
use std::path::Path;
use std::path::PathBuf;

use deno_core::op2;
use deno_runtime::ops::bootstrap::SnapshotOptions;
use deno_runtime::snapshot::create_runtime_snapshot;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
struct ZintlWindowCreateOptions {}

#[derive(Deserialize)]
struct ZintlWindowBounds {}

#[derive(Deserialize)]
struct ZintlWindowSize {}

#[derive(Deserialize)]
struct ZintlWindowPosition {}

#[derive(Deserialize)]
struct ZintlWindowCommandSet {}

#[derive(Serialize)]
struct ZintlWindowCommandEvent {}

#[derive(Serialize)]
struct ZintlWindowLifecycleEvent {}

#[op2]
fn op_zintl_window_create(#[serde] _options: Option<ZintlWindowCreateOptions>) -> u32 {
    0
}

#[op2]
fn op_zintl_window_set_bounds(_window_id: u32, #[serde] _bounds: ZintlWindowBounds) {}

#[op2]
fn op_zintl_window_set_size(_window_id: u32, #[serde] _size: ZintlWindowSize) {}

#[op2]
fn op_zintl_window_set_position(_window_id: u32, #[serde] _position: ZintlWindowPosition) {}

#[op2]
fn op_zintl_window_set_commands(_window_id: u32, #[serde] _commands: ZintlWindowCommandSet) {}

#[op2]
#[serde]
fn op_zintl_window_take_command_event() -> Option<ZintlWindowCommandEvent> {
    None
}

#[op2]
#[serde]
fn op_zintl_window_take_lifecycle_event() -> Option<ZintlWindowLifecycleEvent> {
    None
}

deno_runtime::deno_core::extension!(
    zintl,
    ops = [
        op_zintl_window_create,
        op_zintl_window_set_bounds,
        op_zintl_window_set_size,
        op_zintl_window_set_position,
        op_zintl_window_set_commands,
        op_zintl_window_take_command_event,
        op_zintl_window_take_lifecycle_event,
    ],
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
