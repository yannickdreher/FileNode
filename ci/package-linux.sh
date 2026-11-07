#!/bin/sh
set -eu
test -n "${RELEASE_VERSION:-}" || exit 0
node ci/stamp-version.mjs "$RELEASE_VERSION"
sh scripts/build-linux.sh release
mkdir -p dist/linux
cp "${CARGO_TARGET_DIR:-target}/release/noridrive" "${CARGO_TARGET_DIR:-target}/release/noridrive-core" "${CARGO_TARGET_DIR:-target}/release/noridrive-ui" dist/linux/
cp README.md README.de.md config.example.toml .release/RELEASE-NOTES.md dist/linux/
"${CARGO_TARGET_DIR:-target}/release/noridrive" --version | grep -F "noridrive $RELEASE_VERSION"
tar -czf "dist/noridrive-${RELEASE_VERSION}-linux-x86_64.tar.gz" -C dist/linux .
node ci/release/manifest.mjs "$RELEASE_VERSION" linux
