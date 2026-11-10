#!/bin/sh
# Exercise the extracted bundle layout using a disposable user profile.
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${CARGO_TARGET_DIR:-$project/target}/debug
fixture=$(mktemp -d)
trap 'rm -rf -- "$fixture"' EXIT HUP INT TERM
mkdir -p "$fixture/bundle"
for file in filenode filenode-core filenode-ui filenode-kde filenode-gnome filenode.svg; do
    cp "$output/$file" "$fixture/bundle/"
done
cp -R "$output/locales" "$output/integration" "$fixture/bundle/"
cp "$project/scripts/install-linux.sh" "$fixture/bundle/"
prefix="$fixture/install with spaces"
data="$prefix/share"
sh "$fixture/bundle/install-linux.sh" --prefix "$prefix"
test -L "$prefix/bin/filenode-ui"
test -x "$data/filenode/app/filenode-core"
test -f "$data/nautilus-python/extensions/filenode.py"
test -f "$data/icons/hicolor/scalable/apps/io.github.yannickdreher.FileNode.svg"
test -f "$data/applications/io.github.yannickdreher.FileNode.desktop"
# Execute through the installed symlink to check resource/launcher resolution.
FILENODE_DESKTOP=gnome "$prefix/bin/filenode-ui" --help >/dev/null
"$data/filenode/app/filenode-core" --version
echo 'Linux per-user bundle installation passed.'
