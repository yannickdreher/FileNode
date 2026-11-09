# Run and debug

[Deutsch](development.de.md)

Open the FileNode repository itself in VS Code. The frontend projects and Rust core have separate launch profiles. Build/debug configuration does not automatically connect to a real account: `scripts/prepare-debug.*` creates an ignored `.dev/profile.toml` with a blank username, autostart disabled and deletion propagation off.

## Extensions

The checked-in recommendations use official publishers:

- `rust-lang.rust-analyzer`: Rust language tooling, published by the Rust project.
- `ms-vscode.cpptools`: Microsoft's native debugger (`cppvsdbg` on Windows, `cppdbg`/GDB on Linux).
- `ms-dotnettools.csharp`: Microsoft's C# debugger (`coreclr`) for WinUI.
- `ms-vscode.cmake-tools`: Microsoft's CMake integration for the Linux project.

The earlier `lldb` profiles were removed. They require the separate third-party extension `vadimcn.vscode-lldb`; it is not needed by the new setup. A debugger extension does not install the Rust compiler or C++ SDK.

## Windows

Install Rust using rustup with MSVC, the matching Visual Studio C++ build tools/Windows SDK, and .NET 10 SDK. Visual Studio without its C++ workload is insufficient for Rust MSVC. The WinUI build obtains Windows App SDK build packages through NuGet. Native WinUI development tooling in Visual Studio is optional when using the provided CLI workflow. Do not enable Developer Mode or install certificates merely to compile an unpackaged build.

1. In **Visual Studio Installer**, select **Modify → Desktop development with C++** for the existing installation, including the x64/x86 MSVC tools and a Windows SDK. See the [MSVC prerequisites](https://rust-lang.github.io/rustup/installation/windows-msvc.html).
2. [Install Rust through rustup](https://rust-lang.org/tools/install/) using the MSVC toolchain. Cargo is included; it is not a separate installation.
3. Fully exit and restart VS Code to pick up the updated `PATH`. The first repository build downloads the toolchain pinned in `rust-toolchain.toml`.

```powershell
rustc --version
cargo --version
dotnet --version
powershell.exe -NoProfile -File scripts/build-windows.ps1
```

**FileNode: Windows existing build (no Cargo required)** launches the existing Windows x64 test build at `.dev/filenode-preview/filenode-ui.exe` with F5. It includes the matching Rust worker, so launching it needs neither Cargo nor C++ build tools. This profile opens settings with `.dev/profile.toml`; it does not rebuild source changes and requires the local build with its companion files to exist. Binaries are not checked in. Quit any older instance using the same profile through its tray menu first. Release builds have limited debugging support; Rust breakpoints still require a matching native debug build. The separate output directory avoids locks held by an instance running from the normal build directory.

To refresh this preview using an already compiled Windows x64 core:

```powershell
powershell.exe -NoProfile -File scripts/build-windows.ps1 -Configuration Release -RustCorePath C:/path/to/filenode.exe -OutputDirectory .dev/filenode-preview
```

The script normalizes the final directory separator in the WinUI output path. Without that separator, `App.xbf` can be embedded under the wrong resource name; the project rejects such a direct `OutputPath` instead of producing a broken build.

Run `powershell.exe -NoProfile -File scripts/windows-smoke.ps1` to exercise the existing preview natively: startup, all four settings pages, English/German and quit. It creates its own blank-username profile and connects no account. Results are written to `artifacts/windows-smoke/ui-smoke.json`. This test requires a signed-in Windows desktop session.

When a notification-area host is available, the smoke test also checks the actual HWND frame flags and repeated popup dismissal/reopening in dark and light themes, including queued deactivation. XAML-only screenshots cannot reveal a native non-client border. The tray panel removes the classic `WS_DLGFRAME` left by the non-resizable WinUI presenter separately from its DWM border, and stays out of the taskbar; settings retain their normal caption and taskbar entry.

Foreground activation assertions require Windows to grant the test window foreground access. A test started in the background can be denied that access; the report then marks `repeatedTrayActivation=false` and prints an explicit skip while still checking native frame flags and the remaining UI behavior.

Run `powershell.exe -NoProfile -File scripts/windows-shutdown-smoke.ps1 -Executable .dev/filenode-preview/filenode-ui.exe` for native update shutdown checks. Separate blank-account fixtures verify update consent, cancellation, ordinary close-to-tray, and clean termination of both UI and Rust worker via each window's session messages. The optional real installer upgrade test is restricted to disposable GitHub runners. It does not access the default user profile.

Choose **FileNode: Windows WinUI (Microsoft C# debugger)** and press F5 to debug the UI. It builds Rust and WinUI, prepares a separate debug profile and opens settings. The Rust worker runs as a child; C# breakpoints do not debug Rust. For core breakpoints use **FileNode: Rust core Windows (Microsoft debugger)** with a dedicated test account configured in `.dev/profile.toml`, or attach the Microsoft native debugger to the worker. This profile uses the MSVC build/PDB; GNU cross-build debug info is not a substitute for MSVC debugging.

`-RustCorePath <existing.exe>` lets the build script use a separately compiled core when developing only the UI. On the current workstation a development-only copy can be placed under `.tools/windows-core`; it is ignored and not a dependency of clean checkouts. Do not commit binaries. No claim is made that missing native Rust build prerequisites were installed by building WinUI through NuGet.

For an unsigned local package, run the build script with `-Configuration Release -Package`. Compilation and package generation do not install or publish the app. See [packaging](desktop-frontends.md).

## Linux

On Debian/Ubuntu install the distribution equivalents of:

```sh
sudo apt install build-essential pkg-config libdbus-1-dev cmake ninja-build gdb \
  qt6-base-dev qt6-declarative-dev qml6-module-qtquick qml6-module-qtquick-window \
  qml6-module-qtquick-layouts qml6-module-qtquick-controls qml6-module-qtquick-dialogs \
  qml6-module-qtquick-templates qml6-module-qtqml qml6-module-qtqml-workerscript
sh scripts/build-linux.sh
```

Install Rust 1.98.1 through rustup; the repository pins the toolchain and lockfile. An unlocked Secret Service (for example GNOME Keyring or a compatible KDE Wallet setup) is needed for saved credentials. Qt 6 KDE styling is optional. Wayland/X11 and a StatusNotifier host are needed for normal desktop/tray operation. On GNOME a tray extension may be necessary.

Use **FileNode: Linux Qt (GDB)** for the frontend; it disables the tray for convenient window debugging. Use **FileNode: Rust core Linux (GDB)** for the headless engine with a disposable test profile/account. `rust-gdb` can provide Rust pretty printers for terminal debugging. These profiles use the standard Cargo target directory; custom `CARGO_TARGET_DIR` values require matching launch paths.

## Account-free verification

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo check --locked --no-default-features
```

Tests use temporary files/SQLite, an in-memory remote and loopback HTTP servers. They cover both protocol schemas, conflicts, recovery, failed listings, file locks, settling, budgets, power gates and worker IPC without a live account. Linux UI lifecycle checks are described in [desktop frontends](desktop-frontends.md). A native Windows runner must additionally compile and exercise the WinUI shell and MSIX package before release.
