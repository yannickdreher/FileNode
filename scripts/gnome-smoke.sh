#!/bin/sh
set -eu
profile=$(mktemp -d)
mkdir -p "$profile/Documents" "$profile/Cloud"
cat > "$profile/config.toml" <<EOF
username = ""
language = "de"
[[sync_folders]]
local_folder = '$profile/Documents'
files_on_demand = false
[[sync_folders]]
local_folder = '$profile/Cloud'
files_on_demand = true
EOF
GTK_A11Y=none LANG=C.UTF-8 xvfb-run -a dbus-run-session -- "${CARGO_TARGET_DIR:-target}/debug/filenode-gnome" \
  --core "${CARGO_TARGET_DIR:-target}/debug/filenode-core" --config "$profile/config.toml" \
  --no-tray --self-test artifacts/gnome-desktop
python3 -c 'import json; r=json.load(open("artifacts/gnome-desktop/gnome-smoke.json")); assert r["ready"] and r["folders"]==2 and not r["error"], r'
