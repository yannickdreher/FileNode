#!/bin/sh
# Capture both desktop languages in an isolated X11 display. Never connects an account.
set -eu
mkdir -p artifacts
Xvfb :99 -screen 0 1200x840x24 > /tmp/drive-xvfb.log 2>&1 &
display_pid=$!
app_pid=""
trap 'test -z "$app_pid" || kill "$app_pid" 2>/dev/null || true; kill "$display_pid" 2>/dev/null || true' EXIT
sleep 1
export DISPLAY=:99
export LIBGL_ALWAYS_SOFTWARE=1
for language in en de; do
    "${CARGO_TARGET_DIR:-target}/release/noritec-drive" --language "$language" --no-tray --settings --config /tmp/noritec-drive-preview.toml > /tmp/drive-preview.log 2>&1 &
    app_pid=$!
    sleep 3
    kill -0 "$app_pid"
    import -window root "artifacts/desktop-$language.png"
    kill "$app_pid"
    wait "$app_pid" || true
    app_pid=""
done
