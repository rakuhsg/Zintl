#!/usr/bin/env bash

set -eu
REV="$(cat thirdparty/deno.rev)"
rm -rf thirdparty/deno
git clone https://github.com/denoland/deno.git thirdparty/deno
cd thirdparty/deno
git checkout "$REV"
git apply --check ../../patches/deno/*.patch
git apply ../../patches/deno/*.patch
