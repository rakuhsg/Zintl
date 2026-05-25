#!/usr/bin/env bash

SCRIPT_DIR=$(dirname "$0")

cargo build --manifest-path "$SCRIPT_DIR/../../thirdparty/deno/Cargo.toml" --target-dir "$SCRIPT_DIR/target-deno"
cargo build --manifest-path "$SCRIPT_DIR/Cargo.toml" --target-dir "$SCRIPT_DIR/target"

$SCRIPT_DIR/target-deno/debug/deno run \
    --unstable-webgpu \
    --unstable-ffi \
    --allow-ffi \
    --allow-read \
    "$SCRIPT_DIR/test.ts"

