FROM rust:1-slim-trixie@sha256:f6c4a35d21dbb3c89d7a8d056c193e94fc09037f2b58dd858b09c39a47dd02d7
RUN apt-get update -qq && apt-get install -y --no-install-recommends \
    build-essential pkg-config libdbus-1-dev cmake ninja-build git ca-certificates \
    fuse3 libfuse3-dev qt6-base-dev qt6-declarative-dev \
    libkf6kio-dev libkf6coreaddons-dev extra-cmake-modules \
    qml6-module-org-kde-kirigami qml6-module-org-kde-desktop \
    qml6-module-qtquick qml6-module-qtqml qt6-svg-plugins qml6-module-qtquick-controls qml6-module-qtquick-layouts \
    qml6-module-qtquick-window qml6-module-qtquick-templates \
    qml6-module-qtqml-workerscript qml6-module-qtquick-dialogs \
    python3 python3-gi gir1.2-gtk-4.0 gir1.2-adw-1 python3-nautilus librsvg2-common \
    dbus-x11 xvfb xauth openbox xdotool wmctrl imagemagick python3-dbus && rm -rf /var/lib/apt/lists/*
