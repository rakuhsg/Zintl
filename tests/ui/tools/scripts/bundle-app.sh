#!/bin/sh

set -eu

repository_root=$(git rev-parse --show-toplevel)
ui_test_root="$repository_root/tests/ui"
output_root="$ui_test_root/target"
cargo_output="$output_root/cargo"
app_path="$output_root/ZintlUITestApp.app"
executable_path="$app_path/Contents/MacOS/ZintlUITestApp"

CARGO_TARGET_DIR="$cargo_output" \
    cargo build --manifest-path "$ui_test_root/Cargo.toml"

mkdir -p "$app_path/Contents/MacOS"
cp "$cargo_output/debug/zintl-ui-test-app" "$executable_path"
cp "$ui_test_root/tools/app-template/Info.plist" "$app_path/Contents/Info.plist"

codesign --force --sign - --timestamp=none "$app_path"
