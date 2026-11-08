# Native desktop projects

The repository contains one Rust synchronization implementation and two desktop projects:

| Component | Technology | Responsibility |
| --- | --- | --- |
| `src/lib.rs` and modules | Rust | JMAP, planning, transfers, limits, file safety, state |
| `src/main.rs`, `src/service.rs` | Rust executable | Headless CLI and private desktop worker |
| `apps/windows` | C# / .NET 10 / WinUI 3 | Windows controls, Mica, tray, power, activation, MSIX |
| `apps/linux` | C++ / Qt 6 / QML | Qt controls, KDE integration, tray, D-Bus power events |
| `locales` | Shared JSON catalogs | English/German UI and structured activity messages |

These are build projects in the same GitHub repository. Separate repositories would add release/version coordination without removing any UI maintenance.

## Process lifetime and protocol

Each desktop host launches an adjacent `filenode-core[.exe] --service` with private redirected stdin/stdout. Frames are UTF-8 JSON followed by a newline. The worker's `ready` event declares `protocol: 4` and the password-free configuration. Commands are bounded to 1 MiB; UI event buffers are bounded to 16 MiB. No permanent IPC port is exposed. An explicit browser sign-in temporarily listens on IPv4 loopback for its OAuth callback. Windows uses AppInstance to redirect another launch of the same profile to the existing UI; the Rust store additionally enforces one writer per local folder on both platforms.

Commands: `configure {patch,connect?,password?,interactive?}` (or a complete `config`), `connect {password?,interactive?}`, `pause`, `check`, `power {saver,sleeping,available}`, `quit`. Events: `ready`, `configured`, `running`, `status`, `report`, `activity` (technical log), `account`, `folder`, `file`, `retry`, `error`, `power`, `open_browser`. The worker merges patches against the current profile so rapid edits cannot overwrite other preferences. A sign-in configuration queues connection only after successful saving. Changes to sync rules stop and join the previous worker before automatically continuing; presentation and shared bandwidth/power preferences apply without a restart. Changed accounts require explicit sign-in. Only a successful authenticated JMAP session verifies sign-in. Sync pause retains the verified account state. Folder states are indexed by selected local root; file rows are updated by `(folder, path)` rather than appended for every phase. Byte progress includes streamed payload, while success requires the engine's completed state update. Only Rust accesses the OS credential store. A supplied app password crosses the inherited input pipe, never command-line arguments. UI runtimes can retain managed string allocations; the app does not claim complete secret-memory erasure. The protocol-4 configuration contains a `sync_folders` array of `{local_folder, remote_path}` selections; a `report` includes `failed_folders`. Frontends display generated server paths and use native folder pickers. There is no top-level local-folder setting. See [folder selection](folder-selection.md) and [browser authentication](browser-authentication.md).

Closing windows keeps the host and worker alive. Quit closes the worker cleanly; EOF also stops it. A host crash therefore stops its worker once the pipe closes. This is deliberate per-user application ownership, not an independent OS daemon. `--service` is an internal mode name, not a registered Windows or Linux service. Power state is supplied by the living desktop host; the plain headless CLI has no OS power subscription.

## Windows

`FileNode.Windows.csproj` uses actual Microsoft WinUI controls and the stable Windows App SDK package pinned in the project. UI code is C#; the sync implementation remains Rust. A normal debug build is unpackaged and self-contained. Build with `scripts/build-windows.ps1`; use `-RustCorePath <existing.exe>` to supply a separately built worker. `-Platform ARM64` builds the matching Rust MSVC target when the target/toolchain is installed. ARM64 runtime behavior has not been validated.

`-Configuration Release -Package` creates an unsigned MSIX in `apps/windows/AppPackages`. The project refuses packaging if the Rust executable is missing. `Package.appxmanifest` includes desktop full-trust capability and an opt-in startup task. Packaged startup uses Windows StartupTask; unpackaged startup uses HKCU. Package creation never registers, installs or submits the app.

`scripts/build-windows-installer.ps1 -Version X.Y.Z -SourceDirectory <desktop-output>` compiles `apps/windows/installer.iss` using Inno Setup 6.6+ (available on the hosted windows-2025 runner). It creates a per-user EXE installer under `dist/` with the complete unpackaged payload, Start menu integration and uninstall support. Its `PrivilegesRequired=lowest` setting avoids a UAC request. Startup remains an opt-in app preference; uninstall removes that Run value only when it points to the installed executable. Profiles, credentials and synchronized data are not installer-owned files.

The OID/`-AllowUnsigned` change in 0.2.1 did not enable installation. Windows rejects standalone executable activation with `0x80073D2B`. The unsigned hosted-app route forbids `Executable`/`EntryPoint` activation data; it is not a deployment route for this standalone WinUI app. Use the EXE installer or portable ZIP for unsigned distribution, and trusted signing/Store signing for MSIX. The development manifest no longer includes the hosted-app OID. See [Microsoft's hosted-app restrictions](https://learn.microsoft.com/en-us/windows/uwp/launch-resume/hosted-apps#requirements-for-unsigned-hosted-apps).

`scripts/windows-installer-smoke.ps1` is restricted to disposable GitHub Actions runners. It silently installs into a fresh runner-temp directory, verifies the payload, worker version and Start menu entry, uninstalls, and verifies that an unowned fixture survives. It never launches the account-bearing UI. Both PR checks and release packaging run this deployment test; building an MSIX alone is not considered an installation test.

The manifest identity and publisher are **development placeholders**. Before Store submission, reserve the app in Partner Center, copy its exact identity/publisher into the manifest, add actual store metadata/privacy/support information, validate release architectures, and run native interactive acceptance plus the Windows App Certification Kit. Store certification is not implied by a successful build. The FileNode name has not been trademark-cleared. See [Microsoft publishing guidance](https://learn.microsoft.com/en-us/windows/apps/publish/).

The required Windows jobs run on GitHub-hosted `windows-2025` runners with .NET 10 and the Rust MSVC/C++ prerequisites. It separately builds the unpackaged portable desktop app and the unsigned MSIX, using separate intermediate outputs. Release publication waits for Linux and Windows assets. See [releases](releases.md).

## Linux

Qt 6.4+ is supported. `scripts/build-linux.sh [debug|release]` builds Rust and CMake, placing UI/core/CLI together under the Cargo profile directory. Install the matching distribution runtime modules listed in the development guide; the archive does not bundle Qt. On KDE install the Qt 6 desktop style and platform integration packages supplied by the distribution. The frontend respects explicit Qt style overrides. A single Qt frontend cannot also become GTK/libadwaita on GNOME. GNOME users can run it with Qt styling and an appropriate tray host.

Filesystem subscriptions belong to Rust `notify`, not the Linux system bus. The bus carries logind sleep/resume and power-profiles-daemon profile notifications. Missing power-profile support is shown as unavailable, without preventing standby. No inhibitor is registered. Qt/OS native events wake the UI; no continuous UI repaint loop is required.

`scripts/desktop-smoke.sh` uses isolated D-Bus, Xvfb and a StatusNotifier fixture to check close-to-background, tray activation, settings and quit. `--self-test <artifact-directory>` provides an additional offline Qt UI/power-protocol smoke test. Neither test contacts a JMAP account.
