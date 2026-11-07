# Mit VS Code starten und debuggen

[English](development.md)

Den **Drive-Repository-Ordner** in VS Code öffnen. `.vscode` enthält Erweiterungsempfehlungen, Build-/Test-Aufgaben und vier F5-Startkonfigurationen. Zum Debuggen werden native Debug-Builds mit Symbolen erzeugt. Die optimierten Release-Downloads sind dafür nicht geeignet.

## Voraussetzungen

- VS Code mit **rust-analyzer** (`rust-lang.rust-analyzer`) und **CodeLLDB** (`vadimcn.vscode-lldb`). Die empfohlenen Erweiterungen des Projekts installieren.
- Rust über [rustup](https://rustup.rs/). Das Projekt legt Rust 1.98.1, Clippy, rustfmt und rust-src fest. Nach der Installation VS Code neu öffnen, damit Cargo im PATH verfügbar ist.
- Windows: MSVC-Rust-Toolchain, **Visual Studio 2022 Build Tools** mit **Desktopentwicklung mit C++**, MSVC x64/x86-Werkzeugen und Windows SDK. Eine vorhandene Visual-Studio-Installation ohne C++-Workload genügt nicht. Siehe [offizielle Windows-Voraussetzungen](https://rust-lang.github.io/rustup/installation/windows.html).
- Linux: Build-Abhängigkeiten aus dem README, X11/Wayland und Secret Service für Passwörter. Für das Tray-Symbol muss ein StatusNotifier-Host laufen; GNOME benötigt gegebenenfalls eine AppIndicator-/StatusNotifier-Erweiterung. Das verwendete KSNI-Backend benötigt kein GTK/libappindicator.

In einem neuen VS-Code-Terminal prüfen:

```sh
rustc --version
cargo --version
cargo build --locked
```

Der native Windows-MSVC-Build zum Debuggen ist unabhängig vom Windows-GNU-Cross-Build der Linux-CI.

## F5 verwenden

1. `C:\repos\noritec\drive` beziehungsweise deinen Checkout in VS Code öffnen.
2. Voraussetzungen und empfohlene Erweiterungen installieren.
3. Einen Breakpoint in `src/ui.rs` bei `App::update` oder in `src/engine.rs` bei `sync_with_scanner` setzen.
4. Unter „Ausführen und Debuggen“ **Drive: desktop and settings** auswählen und **F5** drücken.

Vor dem Start werden `.dev/profile.toml` und `.dev/files` angelegt, falls sie fehlen. Ein vorhandenes Debug-Profil bleibt erhalten. Automatische Verbindung, Betriebssystem-Autostart und Löschweitergabe sind deaktiviert. Der Remote-Ordner heißt `Noritec Drive Debug`. Profil und Dateien werden von Git ignoriert. Das normale Benutzerprofil wird nicht verwendet; erst nach dem Eintragen des Kontos und dem Verbinden wird der Server angesprochen. Das Passwort bleibt im Betriebssystem-Schlüsselspeicher.

Weitere Konfigurationen:

- **Drive: background and tray** startet mit `--background`. Nach der ersten Einrichtung im Debug-Profil die automatische Verbindung aktivieren, um den unbeaufsichtigten Start zu prüfen. Ein unkonfiguriertes Profil bleibt sichtbar.
- **Drive: desktop without tray** hält die Oberfläche auf Desktops ohne nutzbaren Infobereich erreichbar.
- **Drive: synchronization integration tests (no account)** startet Tests mit temporären Dateien, SQLite und simuliertem Server. Damit lassen sich Konflikte und Löschschutz ohne echte Kontodaten debuggen.

**F10** führt die nächste Anweisung aus, **F11** springt in eine Funktion, **Shift+F11** aus ihr heraus. **Shift+F5** beendet den Debug-Prozess. Reguläres Schließen des Fensters lässt eine Tray-fähige App weiterlaufen; **Noritec Drive beenden** im Menü beendet sie kontrolliert. Für einen temporären Debug-Build keinen Betriebssystem-Autostart aktivieren.

Ein F5-Debugging auf einem Rechner ohne Rust oder Debugger-Erweiterung ist erst nach deren Installation möglich. Die Konfigurationsdateien installieren diese Voraussetzungen nicht automatisch. Der automatisierte Linux-Desktop-Test und dessen Voraussetzungen stehen in der [englischen Anleitung](development.md#desktop-verification).
