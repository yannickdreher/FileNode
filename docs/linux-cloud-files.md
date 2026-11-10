# Linux Files On-Demand

The development build provides a shared Rust FUSE 3 filesystem, a native GTK 4/libadwaita GNOME frontend, a Qt 6/Kirigami KDE frontend, and separate Nautilus 4/Dolphin 6 extensions. The same JMAP transport, three-way planner, authentication, bandwidth budgets and server trash are used on both desktops. Older published packages do not contain these changes.

## Setup

Use GTK 4.10+, libadwaita 1.5+, Qt 6.4+, Kirigami 6, KDE Frameworks 6 and FUSE 3. The release baseline is Debian 13; Debian 12's previous Qt-only dependencies are insufficient. Saved credentials require an unlocked Secret Service. No systemd service is installed.

Extract the Linux archive and run `sh install-linux.sh` as your ordinary user. It installs the app, launcher, icon and Nautilus extension. Install your distribution's `python3-nautilus`/Nautilus 4 bindings and restart Nautilus. Enable **FileNode availability** in its column preferences. For Dolphin, explicitly run `sudo sh install-linux.sh --system-dolphin` from the extracted package. This optional step installs the matching native plugins in Qt's system plugin directory; restart Dolphin and enable FileNode plugins if necessary. The sync engine always runs as your ordinary user.

`filenode-ui` selects GNOME for GNOME/Unity sessions and KDE otherwise. `FILENODE_DESKTOP=gnome` or `FILENODE_DESKTOP=kde` overrides selection. Both frontends share the profile and English/German catalogs. GNOME keeps running after its window is closed and reopens through the application launcher; GNOME has no default tray. KDE retains its StatusNotifier tray.

For a custom installation location, use `sh install-linux.sh --prefix /absolute/directory`. The application and desktop resources go under `share`, and the launcher goes under `bin` within that prefix. Add its `bin` to your path if needed. The default remains the current user's XDG data directory and `~/.local/bin`.

Choose **Add cloud folder** and select an **empty, dedicated local folder**. Other selections can use full synchronization. FileNode does not mount over existing files. Continue full sync for existing populated folders or use a separate empty cloud folder with an unambiguous server binding. Failed settings restore the last accepted configuration.

## Behavior

- Listings and status queries read metadata only. File opens fetch requested immutable blob ranges in bounded 1 MiB chunks. Completed chunks survive reconnects and restarts. Downloads serialize only the same inode, with up to four FUSE request workers per mount.
- **Always keep on this device** recursively pins and hydrates the selection. New server children inherit a pinned parent. Later sync cycles retry failed pin downloads.
- **Free up space** releases only synchronized cache contents. Dirty/open files are refused. Editing an existing file hydrates untouched contents first; truncating to zero does not download the previous version.
- Nautilus provides a localized column and emblems; Dolphin uses native overlay/action plugins. Availability queries are asynchronous. Commands use a bounded local socket protocol accepting only the owning UID, without credentials.
- The ordinary planner handles conflicts, renames and trash. Local deletions move server objects to trash; server trash moves remove local entries. Own uploads are not downloaded again.

## Lifecycle and recovery

Unlike persistent Windows placeholders, **the Linux namespace requires FileNode to run**, including for pinned files. Pausing retains the mount and cached access while network operations honor pause/power policy. Normal Quit unmounts it: the underlying folder is empty, while namespace and contents remain in the private cache and reappear on restart. A killed process can leave a disconnected mount; unmount that path with `fusermount3 -u <folder>` before restarting. Do not delete the cache to repair a mount.

Before disabling cloud mode or stopping that folder, pin the whole tree, wait for downloads and close open files. FileNode freezes mutations, unmounts, copies and verifies ordinary files without replacing different existing data, and updates synthetic baseline hashes. Cache and server data are retained. Interrupted materialization keeps the old profile/cache for retry.

Cache/namespace data lives under `$XDG_DATA_HOME/filenode/roots/<folder-key>/linux-cloud`, normally `~/.local/share/filenode/roots/<folder-key>/linux-cloud`. SQLite records chunk availability only after bytes are flushed. Dirty is recorded before writes; restart recovers durable lengths before planning. Cache and retained removed-file contents currently have no automatic quota/retention policy. Free-space actions are explicit.

Hard links, symlinks, special files, ACL/alternate-stream replication, automatic Linux storage-pressure eviction and a CloudProviders account/sidebar exporter are not implemented. Live GNOME/Plasma Wayland and real-server multi-device acceptance remain preview checks; file-manager integration does not imply complete OneDrive parity.

## Verification

`ci/linux.Dockerfile` supplies build dependencies. Real tests require `/dev/fuse` and mount permission; isolated Docker tests additionally pass the device and `SYS_ADMIN`. Native mount tests also pass as an ordinary user through `fusermount3`.

```sh
cargo test --locked --lib cloud::linux::tests:: -- --include-ignored --test-threads=1
cargo test --locked --test synchronization linux_cloud_engine -- --ignored --test-threads=1
sh scripts/build-linux.sh debug
sh scripts/gnome-smoke.sh
DRIVE_UI="$PWD/target/debug/filenode-kde" DRIVE_CORE="$PWD/target/debug/filenode-core" \
  dbus-run-session -- sh scripts/desktop-smoke.sh
```

Coverage includes metadata-only listing, nonzero ranges, interrupted hydration, persistent cache, pin/eviction, dirty-file protection, open-unlink behavior, rename, materialization, own-upload idempotence and both trash directions. GUI smoke tests use the real worker without an account. Container X11 tests do not establish interactive Wayland behavior.

References: [FUSE](https://www.kernel.org/doc/html/latest/filesystems/fuse/fuse.html), [libadwaita](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/), [Kirigami](https://develop.kde.org/frameworks/kirigami/), [Nautilus](https://gnome.pages.gitlab.gnome.org/nautilus-python/), [Dolphin overlays](https://api.kde.org/koverlayiconplugin.html).
