#!/bin/sh
# Run under dbus-run-session in an isolated X11 session. No account or network required.
set -eu
mkdir -p artifacts
rm -f /tmp/filenode-tray-endpoint.json
Xvfb :98 -screen 0 1200x900x24 >/tmp/drive-smoke-xvfb.log 2>&1 &
display_pid=$!
export DISPLAY=:98 LIBGL_ALWAYS_SOFTWARE=1
app_pid="" host_pid="" wm_pid=""
cleanup() {
    result=$?
    if [ "$result" -ne 0 ]; then
        import -window root artifacts/failure-linux.png 2>/dev/null || true
        for log in /tmp/drive-smoke.log /tmp/drive-smoke-wm.log /tmp/drive-smoke-xvfb.log /tmp/drive-tray-host.log; do
            test ! -f "$log" || { cp "$log" artifacts/; cat "$log"; }
        done
    fi
    test -z "$app_pid" || kill "$app_pid" 2>/dev/null || true
    test -z "$host_pid" || kill "$host_pid" 2>/dev/null || true
    test -z "$wm_pid" || kill "$wm_pid" 2>/dev/null || true
    kill "$display_pid" 2>/dev/null || true
    exit "$result"
}
trap cleanup EXIT
sleep 1
openbox >/tmp/drive-smoke-wm.log 2>&1 &
wm_pid=$!
python3 scripts/tray-test-host.py host >/tmp/drive-tray-host.log 2>&1 &
host_pid=$!
sleep 1
kill -0 "$wm_pid"
kill -0 "$host_pid"
profile=$(mktemp -d)
"${DRIVE_UI:-${CARGO_TARGET_DIR:-target}/release/filenode-ui}" \
  --core "${DRIVE_CORE:-${CARGO_TARGET_DIR:-target}/release/filenode}" \
  --config "$profile/profile.toml" >/tmp/drive-smoke.log 2>&1 &
app_pid=$!
sleep 3
kill -0 "$app_pid"
test -f /tmp/filenode-tray-endpoint.json
import -window root artifacts/status-linux.png
window=$(xdotool search --onlyvisible --name '^FileNode$' | head -1)
# Send the window manager's close request without depending on its key bindings.
wmctrl -ic "$(printf '0x%x' "$window")"
attempt=0
while xdotool search --onlyvisible --name '^FileNode$' >/dev/null && [ "$attempt" -lt 8 ]; do
    sleep 1
    attempt=$((attempt + 1))
done
kill -0 "$app_pid"
if xdotool search --onlyvisible --name '^FileNode$' >/dev/null; then echo 'Close did not hide status window' >&2; exit 1; fi
python3 scripts/tray-test-host.py activate
sleep 1
xdotool search --onlyvisible --name '^FileNode$' >/dev/null
python3 scripts/tray-test-host.py Settings
sleep 1
xdotool search --onlyvisible --name 'FileNode.*Settings' >/dev/null
import -window root artifacts/settings-linux.png
python3 scripts/tray-test-host.py 'Quit FileNode'
sleep 2
if kill -0 "$app_pid" 2>/dev/null; then echo 'Quit did not terminate application' >&2; exit 1; fi
wait "$app_pid"
app_pid=""
echo 'Linux smoke passed: native tray, close-to-background, activation, settings, quit.'
