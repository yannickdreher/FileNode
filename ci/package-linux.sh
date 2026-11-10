#!/bin/sh
set -eu
test -n "${RELEASE_VERSION:-}" || exit 0
node ci/stamp-version.mjs "$RELEASE_VERSION"
sh scripts/build-linux.sh release
mkdir -p dist/linux
for file in filenode filenode-core filenode-ui filenode-kde filenode-gnome filenode.svg; do cp "${CARGO_TARGET_DIR:-target}/release/$file" dist/linux/; done
cp -R "${CARGO_TARGET_DIR:-target}/release/locales" "${CARGO_TARGET_DIR:-target}/release/integration" dist/linux/
cp scripts/install-linux.sh dist/linux/install-linux.sh
chmod +x dist/linux/install-linux.sh
cp README.md README.de.md config.example.toml .release/RELEASE-NOTES.md dist/linux/
cp -R docs dist/linux/docs
"${CARGO_TARGET_DIR:-target}/release/filenode" --version | grep -F "filenode $RELEASE_VERSION"
tar -czf "dist/filenode-${RELEASE_VERSION}-linux-x86_64.tar.gz" -C dist/linux .
node ci/release/manifest.mjs "$RELEASE_VERSION" linux
