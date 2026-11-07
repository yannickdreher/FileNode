#!/bin/sh
# Run under dbus-run-session. Needs Xvfb, Openbox, xdotool, ImageMagick and python3-dbus/gi.
set -eu
mkdir -p artifacts
rm -f /tmp/noritec-drive-tray-endpoint.json
Xvfb :98 -screen 0 1200x840x24 >/tmp/drive-smoke-xvfb.log 2>&1 &
display_pid=$!
export DISPLAY=:98 LIBGL_ALWAYS_SOFTWARE=1
app_pid="" host_pid="" wm_pid=""
trap 'test -z "$app_pid" || kill "$app_pid" 2>/dev/null || true; test -z "$host_pid" || kill "$host_pid" 2>/dev/null || true; test -z "$wm_pid" || kill "$wm_pid" 2>/dev/null || true; kill "$display_pid" 2>/dev/null || true' EXIT
sleep 1
openbox >/tmp/drive-smoke-wm.log 2>&1 &
wm_pid=$!
python3 scripts/tray-test-host.py host >/tmp/drive-tray-host.log 2>&1 &
host_pid=$!
sleep 1
"${CARGO_TARGET_DIR:-target}/release/noritec-drive" --language en --config /tmp/drive-smoke-profile.toml >/tmp/drive-smoke.log 2>&1 &
app_pid=$!
sleep 3
kill -0 "$app_pid"
test -f /tmp/noritec-drive-tray-endpoint.json
import -window root artifacts/status-en.png
# Close the visible status window. The process and tray must survive.
window=$(xdotool search --onlyvisible --name '^Noritec Drive$' | head -1)
xdotool windowactivate --sync "$window" key alt+F4
sleep 1
kill -0 "$app_pid"
if xdotool search --onlyvisible --name '^Noritec Drive$' >/dev/null; then echo 'Close did not hide status window' >&2; exit 1; fi
# A real D-Bus tray activation must restore the hidden application.
python3 scripts/tray-test-host.py activate
sleep 1
xdotool search --onlyvisible --name '^Noritec Drive$' >/dev/null
# Verify the visible status menu as well as the native tray menu.
window=$(xdotool search --onlyvisible --name '^Noritec Drive$' | head -1)
xdotool mousemove --window "$window" 345 28 click 1
sleep 1
import -window root artifacts/status-menu-en.png
xdotool mousemove --window "$window" 310 57 click 1
sleep 2
xdotool search --onlyvisible --name 'Noritec Drive.*Settings' >/dev/null
import -window root artifacts/settings-en.png
python3 scripts/tray-test-host.py Settings
# Quit is distinct from closing a window and must terminate the process.
python3 scripts/tray-test-host.py 'Quit Noritec Drive'
sleep 2
if kill -0 "$app_pid" 2>/dev/null; then echo 'Quit did not terminate application' >&2; exit 1; fi
wait "$app_pid"
app_pid=""
echo 'Desktop smoke passed: tray registration, close-to-background, tray activation, settings and quit.'
