#!/usr/bin/env bash

set -euo pipefail

usage() {
  echo "Usage: $0 [--keep] [vX.Y.Z|X.Y.Z]" >&2
}

keep_checkout=false
if [[ "${1:-}" == "--keep" ]]; then
  keep_checkout=true
  shift
fi

if [[ $# -gt 1 ]]; then
  usage
  exit 2
fi

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_dir/../../../.." && pwd)"
revision="${1:-$(<"$repo_root/thirdparty/deno.rev")}"
if [[ "$revision" != v* ]]; then
  revision="v$revision"
fi

deno_source="$repo_root/thirdparty/deno"
if ! git -C "$deno_source" rev-parse --verify --quiet \
  "$revision^{commit}" >/dev/null 2>&1; then
  deno_source="https://github.com/denoland/deno.git"
fi

check_root="$(mktemp -d "${TMPDIR:-/tmp}/zintl-deno-patch-check.XXXXXX")"
deno_checkout="$check_root/deno"

cleanup() {
  status=$?
  if [[ $status -ne 0 || "$keep_checkout" == true ]]; then
    echo "Temporary checkout retained at: $deno_checkout" >&2
  else
    rm -rf "$check_root"
  fi
}
trap cleanup EXIT

git clone --no-local --branch "$revision" --single-branch \
  "$deno_source" "$deno_checkout"

shopt -s nullglob
patches=("$repo_root"/patches/deno/*.patch)
if [[ ${#patches[@]} -eq 0 ]]; then
  echo "No patches found under $repo_root/patches/deno" >&2
  exit 1
fi

for patch in "${patches[@]}"; do
  echo "Checking $(basename "$patch")"
  git -C "$deno_checkout" apply --check "$patch"
  git -C "$deno_checkout" apply "$patch"
done

git -C "$deno_checkout" diff --check
git -C "$deno_checkout" describe --tags --exact-match HEAD
git -C "$deno_checkout" status --short
echo "All Deno patches apply cleanly to $revision"
