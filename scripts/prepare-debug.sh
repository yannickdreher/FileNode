#!/bin/sh
set -eu
project=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
mkdir -p "$project/.dev/files"
if [ ! -f "$project/.dev/profile.toml" ]; then
    case "$project" in *"'"*) echo 'Choose a workspace path without apostrophes for the generated debug profile.' >&2; exit 1;; esac
    cat > "$project/.dev/profile.toml" <<EOF
language = "en"
server = "https://jmap.example.com"
web_url = "https://mail.example.com"
username = ""
local_folder = '$project/.dev/files'
propagate_deletes = false
autostart = false
EOF
fi
printf 'Debug profile: %s\n' "$project/.dev/profile.toml"
