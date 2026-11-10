#!/bin/sh
# Install the application for this user; Dolphin plugins are an explicit system step.
set -eu
source=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
if [ "${1:-}" = --system-dolphin ]; then
    test "$(id -u)" = 0 || { echo 'Run this explicit plugin step with sudo.' >&2; exit 1; }
    plugins=$(qtpaths6 --query QT_INSTALL_PLUGINS)
    install -Dm755 "$source/integration/filenodeoverlay.so" "$plugins/kf6/overlayicon/filenodeoverlay.so"
    install -Dm755 "$source/integration/filenodeactions.so" "$plugins/kf6/kfileitemaction/filenodeactions.so"
    echo 'Dolphin plugins installed. Close and reopen Dolphin.'
    exit 0
fi
data=${XDG_DATA_HOME:-$HOME/.local/share}
bin="$HOME/.local/bin"
if [ "${1:-}" = --prefix ] && [ "$#" = 2 ]; then
    data="$2/share"
    bin="$2/bin"
elif [ "$#" != 0 ]; then
    echo 'Usage: install-linux.sh [--prefix DIRECTORY | --system-dolphin]' >&2
    exit 2
fi
destination="$data/filenode/app"
mkdir -p "$destination" "$bin" "$data/applications" "$data/nautilus-python/extensions"
for file in filenode filenode-core filenode-ui filenode-kde filenode-gnome filenode.svg; do
    install -m755 "$source/$file" "$destination/$file"
done
mkdir -p "$destination/locales"
install -m644 "$source/locales/en.json" "$source/locales/de.json" "$destination/locales/"
ln -sf "$destination/filenode-ui" "$bin/filenode-ui"
install -m644 "$source/integration/filenode-nautilus.py" "$data/nautilus-python/extensions/filenode.py"
install -Dm644 "$source/filenode.svg" "$data/icons/hicolor/scalable/apps/io.github.yannickdreher.FileNode.svg"
cat > "$data/applications/io.github.yannickdreher.FileNode.desktop" <<EOF
[Desktop Entry]
Type=Application
Name=FileNode
Comment=JMAP Files synchronization
Comment[de]=Dateisynchronisation über JMAP
Exec="$destination/filenode-ui"
Icon=io.github.yannickdreher.FileNode
Terminal=false
Categories=Network;FileTransfer;
StartupNotify=true
EOF
echo "FileNode installed in $destination. Restart Nautilus to load its extension."
echo 'For Dolphin, run sudo ./install-linux.sh --system-dolphin from this extracted package.'
