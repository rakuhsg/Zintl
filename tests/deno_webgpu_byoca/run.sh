#!/usr/bin/env bash

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

cargo build --manifest-path "$SCRIPT_DIR/../../thirdparty/deno/Cargo.toml" --target-dir "$SCRIPT_DIR/target-deno"
cargo build --target-dir "$SCRIPT_DIR/target"

$SCRIPT_DIR/target-deno/debug/deno run \
    --unstable-webgpu \
    --unstable-ffi \
    --allow-ffi \
    --allow-read \
    ./test.ts

