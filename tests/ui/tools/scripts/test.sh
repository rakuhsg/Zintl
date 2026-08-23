#!/bin/sh

set -eu

repository_root=$(git rev-parse --show-toplevel)
ui_test_root="$repository_root/tests/ui"
output_root="$ui_test_root/target"
result_path="$output_root/ZintlUITests.xcresult"

"$ui_test_root/tools/scripts/bundle-app.sh"

if [ -e "$result_path" ]; then
    rm -rf "$result_path"
fi

xcodebuild test \
    -project "$ui_test_root/tools/xcode/ZintlUITests.xcodeproj" \
    -scheme ZintlUITests \
    -destination "platform=macOS" \
    -resultBundlePath "$result_path" \
    "$@"
