# Starten und Debuggen

[English](development.md)

In VS Code direkt den FileNode-Repository-Ordner öffnen. Oberflächen und Rust-Kern besitzen getrennte Startprofile. `scripts/prepare-debug.*` erstellt ein ignoriertes `.dev/profile.toml` mit leerem Benutzernamen, ausgeschaltetem Autostart und deaktivierter Löschweitergabe.

## Erweiterungen

Die Empfehlungen verwenden die offiziellen Herausgeber:

- `rust-lang.rust-analyzer`: Rust-Sprachunterstützung des Rust-Projekts.
- `ms-vscode.cpptools`: Microsoft-Debugger für nativen Code; unter Windows `cppvsdbg`, unter Linux GDB über `cppdbg`.
- `ms-dotnettools.csharp`: Microsoft-C#-Debugger für WinUI.
- `ms-vscode.cmake-tools`: Microsoft-CMake-Unterstützung für Linux.

Die bisherigen `lldb`-Profile wurden ersetzt. Dafür wäre die zusätzliche Drittanbieter-Erweiterung `vadimcn.vscode-lldb` erforderlich gewesen; die neue Konfiguration benötigt sie nicht. Erweiterungen installieren weder Rust noch C++ Build Tools.

## Windows

Benötigt werden Rust mit MSVC über rustup, Visual Studio C++ Build Tools samt Windows SDK und das .NET-10-SDK. Eine Visual-Studio-Installation ohne C++-Workload reicht für Rust MSVC nicht aus. WinUI-Buildpakete werden über NuGet geladen. Die CLI-Befehle benötigen zur bloßen Kompilierung weder Developer Mode noch ein installiertes Testzertifikat.

1. Im **Visual Studio Installer** bei der vorhandenen Installation **Ändern → Desktopentwicklung mit C++** auswählen; MSVC für x64/x86 und ein Windows SDK mitinstallieren. Siehe [MSVC-Voraussetzungen](https://rust-lang.github.io/rustup/installation/windows-msvc.html).
2. [Rust über rustup installieren](https://rust-lang.org/tools/install/) und die MSVC-Toolchain verwenden. Cargo ist enthalten und wird nicht separat installiert.
3. VS Code vollständig beenden und neu öffnen, damit die aktualisierte `PATH`-Variable übernommen wird. Das Repository lädt beim ersten Build die in `rust-toolchain.toml` festgelegte Toolchain.

```powershell
rustc --version
cargo --version
dotnet --version
powershell.exe -NoProfile -File scripts/build-windows.ps1
```

**FileNode: Windows existing build (no Cargo required)** startet mit F5 den bereits vorhandenen Windows-x64-Testbuild aus `.dev/filenode-preview/filenode-ui.exe`. Er enthält den passenden Rust-Worker; Cargo und C++-Buildtools werden dafür nicht benötigt. Das Profil öffnet die Einstellungen mit `.dev/profile.toml`. Es baut keine Quellcodeänderungen und funktioniert erst, wenn dieser lokale Build samt Begleitdateien vorhanden ist; Binärdateien sind nicht im Repository enthalten. Eine noch laufende ältere Instanz mit demselben Profil zuerst über ihr Tray-Menü beenden. Ein Release-Build bietet eingeschränkte Debugging-Möglichkeiten; Rust-Breakpoints benötigen weiterhin einen passenden nativen Debug-Build. Der getrennte Ausgabeordner verhindert, dass eine laufende normale Build-Instanz die Vorschau-Aktualisierung blockiert.

Eine neue Vorschau mit einem bereits gebauten Windows-x64-Kern lässt sich so erzeugen:

```powershell
powershell.exe -NoProfile -File scripts/build-windows.ps1 -Configuration Release -RustCorePath C:/path/to/filenode.exe -OutputDirectory .dev/filenode-preview
```

Das Skript normalisiert den abschließenden Verzeichnistrenner im WinUI-Ausgabepfad. Ohne diesen Trenner kann `App.xbf` unter einem falschen Ressourcennamen eingebettet werden; der Build bricht deshalb bei einem solchen direkten `OutputPath` ab.

Mit `powershell.exe -NoProfile -File scripts/windows-smoke.ps1` lässt sich die vorhandene Vorschau nativ prüfen: Start, alle vier Einstellungsseiten, Deutsch/Englisch und Beenden. Dafür wird ein eigenes Profil mit leerem Benutzernamen angelegt; kein Konto wird verbunden. Das Ergebnis liegt unter `artifacts/windows-smoke/ui-smoke.json`. Der Test benötigt eine angemeldete Windows-Desktopsitzung.

**FileNode: Windows WinUI (Microsoft C# debugger)** baut Rust und WinUI und öffnet mit F5 die Einstellungen im eigenen Debug-Profil. C#-Breakpoints betreffen die Oberfläche. Der Rust-Worker läuft als Kindprozess. Für Rust-Breakpoints **FileNode: Rust core Windows (Microsoft debugger)** mit einem eigenen Testkonto in `.dev/profile.toml` verwenden oder den nativen Microsoft-Debugger an den Worker anhängen. Dafür den MSVC-Build mit PDB verwenden, nicht die GNU-Cross-Build-Debugsymbole.

Wer nur die Oberfläche entwickelt, kann `scripts/build-windows.ps1 -RustCorePath <vorhandene.exe>` verwenden. Eine lokale Entwicklungskopie unter `.tools/windows-core` ist ignoriert und gehört nicht ins Repository. Ein erfolgreicher WinUI-Build über NuGet bedeutet nicht, dass fehlende native Rust-/C++-Voraussetzungen auf dem Rechner installiert wurden.

`-Configuration Release -Package` erstellt ein unsigniertes lokales MSIX. Das Paket wird nicht installiert oder veröffentlicht. Weitere Grenzen stehen in der [Paketierungsbeschreibung](desktop-frontends.md).

## Linux

Rust 1.98.1 über rustup installieren. Unter Debian/Ubuntu werden beispielsweise benötigt:

```sh
sudo apt install build-essential pkg-config libdbus-1-dev cmake ninja-build gdb \
  fuse3 libfuse3-dev libkf6kio-dev libkf6coreaddons-dev extra-cmake-modules \
  python3 python3-gi gir1.2-gtk-4.0 gir1.2-adw-1 python3-nautilus librsvg2-common \
  qml6-module-org-kde-kirigami qml6-module-org-kde-desktop qt6-svg-plugins \
  qt6-base-dev qt6-declarative-dev qml6-module-qtquick qml6-module-qtquick-window \
  qml6-module-qtquick-layouts qml6-module-qtquick-controls qml6-module-qtquick-dialogs \
  qml6-module-qtquick-templates qml6-module-qtqml qml6-module-qtqml-workerscript
sh scripts/build-linux.sh
```

Für gespeicherte Passwörter einen entsperrten Secret Service verwenden. Die neue Linux-Basis ist Debian 13 mit GTK/libadwaita, Kirigami/KF6 und FUSE. GNOME nutzt GTK und den Anwendungsstarter; KDE verwendet Kirigami und StatusNotifier. Siehe [Linux-Einrichtung](linux-cloud-files.md).

**FileNode: Linux KDE (GDB)** startet die Oberfläche zum Debuggen ohne Tray. **FileNode: Rust core Linux (GDB)** startet den Headless-Kern mit einem Testprofil. Diese Profile nutzen das normale Cargo-Zielverzeichnis; bei geändertem `CARGO_TARGET_DIR` die Pfade entsprechend anpassen.

## Tests ohne Konto

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo check --locked --no-default-features
```

Tests verwenden temporäre Dateien/SQLite und simulierte beziehungsweise lokale HTTP-Server. Sie prüfen Konflikte, Wiederherstellung, gesperrte und unfertige Dateien, Bandbreitenbudgets, Energiepausen und das Worker-Protokoll. Ein erfolgreicher Build ersetzt keine interaktive WinUI-/Explorer- oder Store-Abnahme.
