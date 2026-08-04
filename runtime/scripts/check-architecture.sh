#!/bin/sh
set -eu

repo_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
runtime_root="$repo_root/runtime"

if [ -e "$repo_root/Cargo.toml" ]; then
  echo "repository-root Cargo.toml is forbidden" >&2
  exit 1
fi

if [ -e "$repo_root/rust-toolchain.toml" ] || [ -e "$repo_root/rust-toolchain" ]; then
  echo "repository-root Rust toolchain override is forbidden" >&2
  exit 1
fi

if find "$runtime_root" \( -name 'rust-toolchain.toml' -o -name 'rust-toolchain' \) | grep -q .; then
  echo "nested Rust toolchain overrides are forbidden" >&2
  exit 1
fi

if find "$runtime_root" -path '*/src/main.rs' \
  ! -path "$runtime_root/examples/javascript-repl/src/main.rs" | grep -q .; then
  echo "unexpected CLI target outside the explicit JavaScript REPL example" >&2
  exit 1
fi

if grep -Eiq 'JavaScriptCore|\bJSC\b|\bV8\b|reactor-kqueue|kqueue|epoll|io_uring|IOCP' \
  "$runtime_root/crates/runtime-core/Cargo.toml" \
  "$runtime_root/crates/runtime-core/src/lib.rs"; then
  echo "runtime-core contains an engine or backend dependency" >&2
  exit 1
fi

if grep -Eiq 'JavaScriptCore|\bJSC\b|\bV8\b' \
  "$runtime_root/crates/runtime-permission/Cargo.toml" \
  "$runtime_root/crates/runtime-permission/src/lib.rs"; then
  echo "runtime-permission contains an engine dependency" >&2
  exit 1
fi

if find "$runtime_root/crates" -name Cargo.toml -exec \
  grep -Eiq 'reqwest|hyper|tokio|async-process|std::process|libloading|dlopen' {} +; then
  echo "network, process, or native-loading dependency is forbidden in the initial runtime" >&2
  exit 1
fi

if grep -Eiq 'JavaScriptCore|\bJSC\b|\bV8\b' \
  "$runtime_root/crates/runtime-resource/Cargo.toml" \
  "$runtime_root/crates/runtime-resource/src/lib.rs"; then
  echo "runtime-resource contains an engine dependency" >&2
  exit 1
fi

if grep -Eiq 'JavaScriptCore|\bJSC\b|\bV8\b' \
  "$runtime_root/crates/runtime-filesystem/Cargo.toml" \
  "$runtime_root/crates/runtime-filesystem/src/lib.rs"; then
  echo "runtime-filesystem contains an engine dependency" >&2
  exit 1
fi

echo "architecture dependency checks passed"
