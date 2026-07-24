#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR=$(cd -- "$(dirname -- "$0")" && pwd)

cargo build --manifest-path "$SCRIPT_DIR/../../thirdparty/deno/Cargo.toml" --target-dir "$SCRIPT_DIR/target-deno"
swift build --package-path "$SCRIPT_DIR"

"$SCRIPT_DIR/target-deno/debug/deno" run \
    --unstable-webgpu \
    --unstable-ffi \
    --allow-ffi \
    --allow-read \
    "$SCRIPT_DIR/test.ts" \
    "$@"
