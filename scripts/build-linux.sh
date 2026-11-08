#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project"
profile=${1:-debug}
case "$profile" in debug) cargo build --locked; type=Debug ;; release) cargo build --locked --release; type=Release ;; *) echo 'Usage: build-linux.sh [debug|release]' >&2; exit 2 ;; esac
output="${CARGO_TARGET_DIR:-target}/$profile"
cmake -S apps/linux -B "$output/linux-ui" -DCMAKE_BUILD_TYPE="$type"
cmake --build "$output/linux-ui" --parallel 2
cp "$output/linux-ui/filenode-ui" "$output/filenode-ui"
cp "$output/filenode" "$output/filenode-core"
