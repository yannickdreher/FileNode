#!/bin/sh
# Run under dbus-run-session in an isolated X11 session. No account or network required.
set -eu
mkdir -p artifacts
rm -f /tmp/noridrive-tray-endpoint.json
Xvfb :98 -screen 0 1200x900x24 >/tmp/drive-smoke-xvfb.log 2>&1 &
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
profile=$(mktemp -d)
"${DRIVE_UI:-${CARGO_TARGET_DIR:-target}/release/noridrive-ui}" \
  --core "${DRIVE_CORE:-${CARGO_TARGET_DIR:-target}/release/noridrive}" \
  --config "$profile/profile.toml" >/tmp/drive-smoke.log 2>&1 &
app_pid=$!
sleep 3
kill -0 "$app_pid"
test -f /tmp/noridrive-tray-endpoint.json
import -window root artifacts/status-linux.png
window=$(xdotool search --onlyvisible --name '^NoriDrive$' | head -1)
xdotool windowactivate --sync "$window" key alt+F4
sleep 1
kill -0 "$app_pid"
if xdotool search --onlyvisible --name '^NoriDrive$' >/dev/null; then echo 'Close did not hide status window' >&2; exit 1; fi
python3 scripts/tray-test-host.py activate
sleep 1
xdotool search --onlyvisible --name '^NoriDrive$' >/dev/null
python3 scripts/tray-test-host.py Settings
sleep 1
xdotool search --onlyvisible --name 'NoriDrive.*Settings' >/dev/null
import -window root artifacts/settings-linux.png
python3 scripts/tray-test-host.py 'Quit NoriDrive'
sleep 2
if kill -0 "$app_pid" 2>/dev/null; then echo 'Quit did not terminate application' >&2; exit 1; fi
wait "$app_pid"
app_pid=""
echo 'Linux smoke passed: native tray, close-to-background, activation, settings, quit.'
