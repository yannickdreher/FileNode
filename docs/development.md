# Run and debug with VS Code

[Deutsch](development.de.md)

Open the **Drive repository folder**, not its parent directory. The checked-in `.vscode` folder provides rust-analyzer and CodeLLDB recommendations, build/test tasks and four F5 launch configurations. These use native debug builds with symbols, not the stripped release downloads.

## Prerequisites

- VS Code with `rust-lang.rust-analyzer` and `vadimcn.vscode-lldb` installed. Accept the workspace recommendations or install them from the Extensions panel.
- Rust through [rustup](https://rustup.rs/). The repository pins Rust 1.98.1, Clippy, rustfmt and rust-src. Reopen VS Code after installing Rust so it inherits Cargo's PATH.
- Windows: the MSVC Rust host toolchain, Visual Studio 2022 Build Tools with **Desktop development with C++ / MSVC x64/x86 tools** and a Windows SDK. A Visual Studio installation without the C++ workload is not sufficient. Follow the [official Rust Windows requirements](https://rust-lang.github.io/rustup/installation/windows.html).
- Linux: the development dependencies listed in the README, a working X11/Wayland desktop, and Secret Service for account credentials. A StatusNotifier host is needed for the tray; GNOME may require an AppIndicator/StatusNotifier extension. The KSNI tray backend does not require GTK or libappindicator libraries.

Check in a fresh VS Code terminal:

```sh
rustc --version
cargo --version
cargo build --locked
```

Toolchain versions are shared by the native developer build and CI. Linux-to-Windows GNU cross-compilation in CI is separate from the recommended native Windows MSVC debugging setup.

## F5

1. Open `C:\repos\noritec\drive` (or your checkout) in VS Code.
2. Install the recommended extensions and the prerequisites above.
3. Place a breakpoint in `src/ui.rs` (`App::update`) or `src/engine.rs` (`sync_with_scanner`).
4. Choose **Drive: desktop and settings** in Run and Debug and press **F5**.

The prelaunch task creates `.dev/profile.toml` and `.dev/files` if missing. Existing debug profiles are preserved. Automatic connection, OS autostart and deletion propagation are disabled; the remote folder is named `Noritec Drive Debug`. The profile and files are ignored by Git. A normal F5 run does not use the regular user profile or contact a server until you configure an account and connect. The password remains in the OS keyring, never in `launch.json`.

Other configurations:

- **Drive: background and tray** starts with `--background`. Enable auto-connect in the debug profile after initial account setup to exercise unattended startup. An unconfigured profile stays visible.
- **Drive: desktop without tray** keeps the window accessible on a desktop without a tray host.
- **Drive: synchronization integration tests (no account)** runs disposable filesystem/SQLite/mock-server tests. Use a breakpoint in the planner or engine to inspect conflicts and deletion protection without real data.

Use **F10** to step over, **F11** to step into, **Shift+F11** to step out, and **Shift+F5** to stop the debug process. Normal window closing keeps a tray-enabled app running; choose **Quit Noritec Drive** from the menu to exit gracefully. Do not enable OS autostart for a temporary debug binary.

The [CodeLLDB manual](https://github.com/vadimcn/codelldb/blob/master/MANUAL.md#cargo-support) documents Cargo-based launch configurations. A built-in task also runs `cargo test --locked --all-targets` and another runs Clippy.

## Desktop verification

For an isolated Linux GUI smoke test, install `xvfb xauth openbox xdotool imagemagick dbus-x11 python3-dbus python3-gi libgl1-mesa-dri libxkbcommon-x11-0`, then run:

```sh
cargo build --locked --release
dbus-run-session -- sh scripts/desktop-smoke.sh
```

The test creates a disposable D-Bus StatusNotifier host and X11 display. It verifies actual tray registration, closing to background, activation from the tray, a separate settings viewport, and explicit quit. It never connects an account. Screenshots go to ignored `artifacts/`. CI runs the same smoke test; it does not substitute for a native Windows shell usability check.
