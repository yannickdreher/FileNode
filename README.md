# Noritec Drive

[Deutsch](README.de.md) · English

A Rust desktop client for **Stalwart JMAP Files**, with Windows and Linux builds and an English/German interface. This is a **0.1 preview**: a functioning full-file sync client, with automated fault tests, not a production-proven replacement for OneDrive.

## What works

- Bidirectional file and directory synchronization into a dedicated `Noritec Drive` remote folder. Other top-level Bulwark folders are outside this profile.
- SHA-256 content comparisons and a persistent three-way baseline in SQLite. File timestamps alone never decide whether content changed.
- Concurrent edits keep both versions: the server version takes the original path, the local version becomes a sibling named `name (conflict <id>).ext` and uploads on the following scan. Type conflicts preserve the local file/tree before rebuilding the server version.
- Remote renames follow stable FileNode IDs; unambiguous local file renames with unchanged content update remote metadata. More complex local renames fall back to transfer/recovery behavior.
- Deletions require explicit opt-in. Local removals move remote entries into `.noritec-drive-trash`; remote removals move local entries into `.noritec-drive/history/<id>/`. Remote content replacement also preserves a recovery copy first. No `FileNode/destroy` requests are issued.
- Global `ifInState` preconditions protect remote mutations. Incomplete scans, missing pinned remote roots, unsupported paths, changed files and mass deletions stop the operation instead of applying a partial destructive plan.
- Single writer per local folder, durable operation journal, reinspection after ambiguous requests, bounded network timeouts, GUI retry with exponential backoff, resumable downloads and streamed staged uploads.
- Credentials use Windows Credential Manager or Linux Secret Service; HTTPS only, with same-origin discovery/API/upload/download enforcement.
- English and German catalogs, automatic system-language detection, a live language switch, and persisted profile preference. Internal code, diagnostics, protocol values and conflict file names are English.

## Start

Download the portable archive from the latest successful GitLab pipeline's **test-and-package** artifacts. Extract and run `noritec-drive.exe` on Windows or `./noritec-drive` on Linux. Windows builds are currently unsigned. Linux x86-64 builds target Debian 12/glibc 2.36 or newer and require a graphical X11/Wayland session, OpenGL, D-Bus, `libxkbcommon-x11-0` for X11, and an unlocked Secret Service (for example GNOME Keyring or KDE Wallet with Secret Service enabled).

1. Enter the JMAP server, email/username, and a **Stalwart app password**. The default server is `https://jmap.example.com`.
2. Choose a local folder. It is created if needed. Use a separate folder for each server/account/profile.
3. Keep deletion propagation disabled for the initial review. Click **Connect & sync**. Initial matching paths with different contents keep both versions.
4. After reviewing the result, optionally enable deletion propagation while paused. Files are fully downloaded; ensure sufficient disk space for both data and recovery copies.

The client discovers the JMAP file account. For ambiguous discovery, configure `account_id` in the profile. Authentication currently uses app passwords over HTTPS, not an interactive authentik browser login.

### Configuration and headless usage

The GUI writes a password-free `config.toml` in the OS user configuration directory (`%APPDATA%\Noritec\Drive\config\` on Windows; `$XDG_CONFIG_HOME/drive/` or `~/.config/drive/` on Linux). Use `--config <path>` for another profile. [config.example.toml](config.example.toml) lists the options; use an existing absolute local folder for headless mode.

```sh
noritec-drive --language en
noritec-drive --language de
noritec-drive --headless --config /path/profile.toml --dry-run
noritec-drive --headless --config /path/profile.toml --once --language en
```

Headless mode reads the app password from the OS keyring, or `NORITEC_DRIVE_PASSWORD` when supplied by your secret manager. Never put the password in the command line or TOML. A build with `--no-default-features` only supports headless mode and requires that environment variable. CLI help, JSON plans and technical diagnostics remain English; human-readable synchronization messages follow the selected language.

`--dry-run` reads local files and remote state, and may create/update local profile metadata; it does not change user files or remote objects. `--once` completes any conflict/rename follow-up scans before exiting. Headless network errors return a nonzero exit status; a service supervisor can restart the process. The GUI retries automatically.

## Recovery and limits

Local recovery copies remain under `.noritec-drive/history`. Locate the desired version there and copy it back into the sync folder. Remote recovery copies remain in `.noritec-drive-trash` inside the remote sync folder and can be moved/copied back using a JMAP file client such as Bulwark. Stop sync before manually restoring. Recovery folders have no automatic retention cleanup and consume space/quota. These copies do not replace independent backups; another client may permanently delete remote blobs.

Never remove `.noritec-drive` to “fix” a profile: it contains the baseline, journal, resumable transfers and local history. If the pinned remote root disappears, the client deliberately stops; inspect/recover the remote folder before changing the profile.

The preview polls every 30 seconds by default and rehashes local contents. It uses JMAP `FileNode/changes` after the first snapshot, but does not have native filesystem watching, push events or block-level transfers. Transfers are serial. Default bounds are 1 GiB per file, 100,000 entries, 200 UTF-8 bytes per file name and 240 bytes per relative path. Windows-reserved names, case/normalization collisions, symlinks/reparse points and special files stop the scan. `.git`, `.noritec-drive` and `.noritec-drive-trash` are excluded. File permissions, ownership, ACLs, alternate streams, hard-link identity and timestamps are not synchronized. Case-only renames on case-insensitive filesystems are not supported reliably yet.

**Not yet implemented:** OneDrive-style files on demand/placeholders, native Explorer integration, Linux virtual filesystem, tray/autostart, browser OAuth/SSO, multi-account management UI, selective sync, shared-folder permissions, conflict/restore UI, bandwidth controls, signed installers and automatic updates. A live Stalwart account, interrupted-power scenarios and large multi-device workloads still require acceptance testing. A file can change in the small interval between final verification and rename; the preview does not claim filesystem transactions against other applications.

Wire adapters cover Stalwart **0.16 `nodeType`/`type`** and the newer **`@type`/`mediaType`** schema. Tests exercise both, including actual HTTP serialization. The FileNode specification is still a draft; server-version compatibility must be verified against a test account before enabling production sync.

## Develop

Code, identifiers, comments and technical documentation use English. Add UI text to **both** `locales/en.json` and `locales/de.json`; use structured messages for worker events so past activity can switch language without changing paths. Catalog tests check matching keys and placeholders.

Rust 1.98.1 is pinned, with dependencies in `Cargo.lock`. On Debian/Ubuntu install `pkg-config libdbus-1-dev libxkbcommon-dev libwayland-dev libegl1-mesa-dev libx11-dev libxi-dev libxcursor-dev libxrandr-dev` before building the desktop client.

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo check --locked --no-default-features
cargo run --locked -- --language en
cargo build --locked --release
```

The GitLab pipeline checks formatting, Clippy and tests, then packages Linux x86-64 and Windows x86-64 GNU builds. Native Windows builds can use the MSVC Rust toolchain with Visual Studio C++ Build Tools. Automated tests use disposable local folders, SQLite and simulated/loopback JMAP servers; no real accounts are accessed. See [architecture](docs/architecture.md) for invariants and remaining work.

Protocol references: [JMAP core (RFC 8620)](https://www.rfc-editor.org/rfc/rfc8620), [FileNode draft](https://datatracker.ietf.org/doc/html/draft-ietf-jmap-filenode-18), [Stalwart file storage](https://stalw.art/docs/collaboration/file-storage/).
