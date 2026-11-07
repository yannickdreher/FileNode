#!/bin/sh
# Offline bilingual account-settings screenshots; never launches a browser or connects an account.
set -eu
mkdir -p artifacts
Xvfb :99 -screen 0 1200x1000x24 >/tmp/drive-preview-xvfb.log 2>&1 &
display_pid=$!
trap 'kill "$display_pid" 2>/dev/null || true' EXIT
export DISPLAY=:99 LIBGL_ALWAYS_SOFTWARE=1
sleep 1
profile=$(mktemp -d)
for language in en de; do
    cat > "$profile/profile.toml" <<EOF
language = "$language"
username = ""
auth_mode = "browser"
oauth_issuer = "https://auth.example.org/application/o/mail/"
oauth_client_id = "example-public-client"
oauth_redirect_port = 54823
autostart = false
EOF
    "${DRIVE_UI:-${CARGO_TARGET_DIR:-target}/release/noridrive-ui}" \
      --core "${DRIVE_CORE:-${CARGO_TARGET_DIR:-target}/release/noridrive}" \
      --config "$profile/profile.toml" --no-tray --self-test "artifacts/preview-$language" --self-test-account
done
