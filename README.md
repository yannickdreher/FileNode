# FileNode

English · [Deutsch](README.de.md) · [Website](https://yannickdreher.github.io/FileNode/) · [Downloads](https://github.com/yannickdreher/FileNode/releases)

A JMAP Files synchronization client for Windows and Linux. The shared synchronization engine is Rust. Windows has a dedicated **WinUI 3/C#** frontend; Linux has a separate **Qt 6/C++/QML** frontend. UI text is English and German; code and technical diagnostics are English. Version 0.1 remains a preview.

## Desktop behavior

The application runs in the signed-in user's session with a tray icon, a compact activity window and a separate settings window. Closing a window keeps synchronization running. Quit stops the application and its Rust worker. No Windows system service or systemd unit is required or installed. Linux needs a StatusNotifier host; GNOME may require an AppIndicator extension. Without a tray, keep the status window open or minimized.

Both frontends share `locales/en.json` and `locales/de.json`, the configuration format, OS credential storage and all sync decisions. Linux uses KDE desktop controls when Qt's KDE style/integration packages are available, and otherwise system Qt styling. It does not provide GTK/libadwaita controls on GNOME.

## Features

- Bidirectional full-file synchronization, SHA-256 comparison, SQLite three-way baselines and a durable operation journal.
- Both copies survive conflicts; stable remote IDs support remote renames, and unambiguous local file renames preserve server identity.
- Deletions are opt-in, with mass-deletion limits. Old local versions go to `.filenode/history`; remote versions go to `.filenode-trash`. Remote `FileNode/destroy` is never called.
- Native file events (`notify`: inotify on Linux, ReadDirectoryChangesW on Windows), debouncing, event-invalidated content hashes, periodic full verification and JMAP `FileNode/changes`.
- Separate upload/download payload caps in KiB/s, editable while running. Zero disables the cap.
- Busy files are deferred; changed files must settle across observations before upload. Unrelated top-level file groups can still sync.
- Desktop power notifications pause work during energy saver/standby; the energy saver preference can be disabled. Resume causes fresh verification. The application never inhibits sleep.
- HTTPS, origin-checked endpoints and app passwords in Windows Credential Manager or Linux Secret Service.

## Build and start

See [development](docs/development.md) and [desktop architecture/packaging](docs/desktop-frontends.md).

Windows, with Rust MSVC, C++ build tools, .NET 10 and the Windows SDK:

```powershell
powershell.exe -NoProfile -File scripts/build-windows.ps1
& apps/windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/filenode-ui.exe
# Create an unsigned local MSIX; this does not install or submit it:
powershell.exe -NoProfile -File scripts/build-windows.ps1 -Configuration Release -Package
```

Linux, with Qt 6.4+ development/runtime modules and a compiler:

```sh
sh scripts/build-linux.sh
./target/debug/filenode-ui
```

Pull requests run Linux and native Windows checks on GitHub Actions. Merges into `main` then plan a semantic-release version, build both platforms and publish a GitHub release with change notes, binary downloads and checksums. Hosted Windows runners need no separate runner installation. GitHub Pages deploys the English/German website. See [releases](docs/releases.md).

## Connect

In settings, configure the JMAP server and username. Defaults are `https://jmap.example.com` for JMAP and `https://mail.example.com` for the web client. Choose **Browser / single sign-on** for OAuth Authorization Code + PKCE or **App password**, then select **Sign in**. Select a dedicated local folder; leave deletion propagation disabled initially. Use a separate folder for each account/profile. See [browser authentication](docs/browser-authentication.md) for server prerequisites and external OIDC providers.

On the Account page, **Sign in** saves the entered settings before authentication. The remote target is automatic: selecting `C:\Users\yannick\Desktop` synchronizes into the top-level remote folder `Desktop`. There is no remote-folder field or account-root selection. Existing local state remains bound to its original remote folder ID; if an older profile used a different name, choose a fresh local folder rather than deleting its state.


A configured account resumes synchronization when FileNode starts. There is no separate startup-resume setting. Browser authentication reuses saved tokens; an interactive browser login still requires the user's sign-in action.

Profiles contain no passwords. Default paths: Windows `%APPDATA%\FileNode\config\config.toml`; Linux `$XDG_CONFIG_HOME/filenode/config.toml` or `~/.config/filenode/config.toml`. Use `--config <path>` for a different profile. See [example configuration](config.example.toml). If discovery finds multiple file accounts, specify `account_id`.

```sh
filenode --headless --config /absolute/profile.toml --dry-run
filenode --headless --config /absolute/profile.toml --once --language en
```

Headless mode requires an existing absolute local folder and saved credentials. Browser-mode profiles reuse/refresh the desktop session without opening a browser. App-password profiles use the OS keyring or `FILENODE_PASSWORD` supplied by a secret manager. Never put passwords in command-line arguments or TOML. `--dry-run` can create private local metadata but changes no user files or remote objects. `--once` waits for settling and returns an error if files remain busy/changing beyond twice the settling interval plus 30 seconds; desktop mode continues retrying. Headless mode does not subscribe to desktop power notifications.

## Limits and recovery

This is a full-file preview, not a complete OneDrive replacement. Files on demand, Explorer placeholders, block transfers, remote push, selective sync, multiple-account UI, conflict/restore UI, signed releases and automatic updates remain open. Transfers are serial. Defaults include a 1 GiB file limit, 100,000 entries, 200 UTF-8 bytes per filename and 240 per relative path. Symlinks/reparse points, unsafe names and case/normalization collisions stop scanning. Permissions, ownership, ACLs, alternate streams and timestamps are not synchronized. Case-only renames are not reliably supported.

The complete namespace is still enumerated; cached hashes reduce content reads, not directory-walk complexity. Watcher loss, manual checks, reconnects and periodic verification rehash contents. Settling and locks reduce races but cannot prove that every application has finished writing. Unix locks are advisory. Payload caps exclude TLS/HTTP overhead and socket buffering. [Architecture](docs/architecture.md) describes these boundaries and acceptance work.

Never remove `.filenode` to repair a profile: it holds the baseline, journal, transfers and recovery history. Pause sync before restoring copies. Local recovery lives under `.filenode/history`; remote recovery under `.filenode-trash` can be accessed through a JMAP client such as Bulwark. Recovery has no automatic retention policy and consumes space/quota. Maintain independent backups.

Adapters cover Stalwart 0.16 `nodeType`/`type` and the newer `@type`/`mediaType` FileNode schema. Loopback tests verify both; live Stalwart compatibility, interrupted-power behavior and large multi-device workloads still need acceptance testing.

Protocol references: [JMAP core](https://www.rfc-editor.org/rfc/rfc8620), [FileNode draft](https://datatracker.ietf.org/doc/html/draft-ietf-jmap-filenode-18), [Stalwart file storage](https://stalw.art/docs/collaboration/file-storage/).
