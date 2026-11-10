#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project"
profile=${1:-debug}
case "$profile" in debug) cargo build --locked; type=Debug ;; release) cargo build --locked --release; type=Release ;; *) echo 'Usage: build-linux.sh [debug|release]' >&2; exit 2 ;; esac
output="${CARGO_TARGET_DIR:-target}/$profile"
cmake -S apps/linux -B "$output/linux-ui" -DCMAKE_BUILD_TYPE="$type"
cmake --build "$output/linux-ui" --parallel 2
cp "$output/linux-ui/kde/filenode-kde" "$output/filenode-kde"
cp apps/linux/filenode-ui "$output/filenode-ui"
cp apps/linux/gnome/filenode-gnome "$output/filenode-gnome"
chmod +x "$output/filenode-ui" "$output/filenode-gnome"
mkdir -p "$output/locales" "$output/integration"
cp locales/en.json locales/de.json "$output/locales/"
cp apps/linux/filenode.svg "$output/filenode.svg"
cp apps/linux/integration/filenode-nautilus.py "$output/integration/"
cp "$output/linux-ui/integration/filenodeoverlay.so" "$output/linux-ui/integration/filenodeactions.so" "$output/integration/"
cp "$output/filenode" "$output/filenode-core"
