# NoriDrive

Deutsch · [English](README.md)

JMAP-Dateisynchronisation für Windows und Linux mit gemeinsamem Rust-Kern. Windows verwendet ein eigenes **WinUI-3-/C#-Projekt**, Linux ein getrenntes **Qt-6-/C++-/QML-Projekt**. Die Oberfläche ist Deutsch/Englisch, Code und technische Diagnosen sind Englisch. Version 0.1 bleibt eine Preview.

## Hintergrundbetrieb

Die App läuft im angemeldeten Benutzerkontext mit Tray-Icon, kompakter Aktivitätsansicht und eigenem Einstellungsfenster. Geschlossene Fenster beenden den Abgleich nicht. Erst Beenden stoppt die Anwendung und den Rust-Worker. Ein Windows-Systemdienst oder eine systemd-Unit wird nicht installiert. Unter Linux ist ein StatusNotifier-Host nötig, unter GNOME gegebenenfalls eine AppIndicator-Erweiterung. Ohne Tray das Statusfenster geöffnet oder minimiert lassen.

Beide Oberflächen teilen Übersetzungen, Profilformat und sämtliche Sync-Regeln. Unter KDE nutzt Qt bei vorhandenen Integrationspaketen den KDE-Desktop-Stil. Unter GNOME ist die Linux-Oberfläche weiterhin Qt und verwendet keine GTK-/libadwaita-Steuerelemente.

## Funktionen

- Vollständige lokale Dateien, bidirektionaler Abgleich, SHA-256-Inhaltsvergleich, SQLite-Dreiwege-Abgleich und dauerhaftes Journal.
- Bei Konflikten bleiben beide Fassungen erhalten. Remote-Umbenennungen nutzen stabile IDs; eindeutige lokale Dateiumbenennungen erhalten die Server-Identität.
- Löschweitergabe erst nach Aktivierung, mit Schutz vor Massenlöschungen. Vorherige Fassungen liegen lokal in `.noritec-drive/history` und remote in `.noritec-drive-trash`. Kein `FileNode/destroy`.
- Native Dateisystem-Ereignisse unter Windows und Linux, Bündelung von Änderungen, Hash-Cache, regelmäßige vollständige Kontrolle und JMAP `FileNode/changes`.
- Upload und Download getrennt in KiB/s begrenzen, auch während des Betriebs. Null bedeutet unbegrenzt.
- Gesperrte Dateien werden zurückgestellt; geänderte Dateien müssen über mehrere Beobachtungen stabil bleiben. Andere oberste Dateigruppen können weiter synchronisieren.
- Desktop-Energieereignisse pausieren Scans und Übertragungen bei Energiesparen/Standby. Die Energiesparoption ist abschaltbar; nach dem Aufwachen wird neu geprüft. Die App verhindert den Standby nicht.
- App-Passwörter im Windows Credential Manager oder Linux Secret Service, HTTPS und Prüfung der Endpoint-Origin.

## Bauen und starten

Die [deutsche Entwicklungsanleitung](docs/development.de.md) beschreibt VS Code und Voraussetzungen. [Projektstruktur und Paketierung](docs/desktop-frontends.md) sind Englisch dokumentiert.

Windows mit Rust MSVC, C++ Build Tools, .NET 10 und Windows SDK:

```powershell
powershell.exe -NoProfile -File scripts/build-windows.ps1
& apps/windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/noridrive-ui.exe
# Unsigniertes lokales MSIX erstellen, ohne Installation oder Veröffentlichung:
powershell.exe -NoProfile -File scripts/build-windows.ps1 -Configuration Release -Package
```

Linux mit Qt 6.4+ samt Entwicklungs- und QML-Laufzeitmodulen:

```sh
sh scripts/build-linux.sh
./target/debug/noridrive-ui
```

Merges nach `main` führen die Tests aus; anschließend plant semantic-release Version und Änderungsprotokoll. Eine serialisierte Unterpipeline baut Linux und die native Windows-App und veröffentlicht danach Tag, GitLab-Release und dauerhafte Binärdownloads. Der Windows-Job benötigt einen Runner mit Tag `windows`; bis dieser eingerichtet ist, wartet die Veröffentlichung auf diesen Job. Siehe [Releases](docs/releases.md).

## Verbinden

Unter Einstellungen Server und Benutzername eingeben. Vorgaben sind `https://jmap.example.com` für JMAP und `https://mail.example.com` für den Webclient. Als Anmeldemethode **Browser / Single Sign-on** für OAuth Authorization Code mit PKCE oder **App-Passwort** auswählen und anschließend **Anmelden** drücken. Einen eigenen lokalen Ordner wählen, die Löschweitergabe zunächst ausgeschaltet lassen. Pro Konto/Profil einen getrennten Ordner verwenden. Die [Browser-Anmeldedokumentation](docs/browser-authentication.md) beschreibt Server-Voraussetzungen und externe OIDC-Anbieter.

Auf der Kontoseite speichert **Anmelden** die Eingaben vor der Authentifizierung. Das Remote-Ziel entsteht automatisch: Der lokale Ordner `C:\Users\yannick\Desktop` wird in den Remote-Ordner `Desktop` auf der obersten Ebene synchronisiert. Es gibt kein Remote-Ordnerfeld und keine Auswahl des Kontostammverzeichnisses. Vorhandene lokale Zustände bleiben an ihre ursprüngliche Remote-Ordner-ID gebunden. Verwendete ein älteres Profil einen anderen Namen, einen neuen lokalen Ordner wählen; den gespeicherten Zustand nicht löschen.

Beim Umbenennen zu NoriDrive bleiben vorhandene Profilpfade, der Credential-Dienst und interne Zustands-/Sicherungsverzeichnisse erhalten. Bereits konfigurierte lokale und Remote-Ordner werden nicht verschoben oder umbenannt. Das GitLab-Repository heißt `noritec/noridrive`.

Ein eingerichtetes Konto setzt die Synchronisation beim Start von NoriDrive fort. Dafür gibt es keine separate Startoption. Die Browser-Authentifizierung verwendet gespeicherte Tokens; eine interaktive Browser-Anmeldung wird weiterhin nur über die Anmeldeaktion gestartet.

Passwortfreie Profile liegen unter Windows in `%APPDATA%\Noritec\Drive\config\config.toml`, unter Linux in `$XDG_CONFIG_HOME/drive/config.toml` beziehungsweise `~/.config/drive/config.toml`. `--config <Pfad>` wählt ein anderes Profil. Optionen stehen in [config.example.toml](config.example.toml). Bei mehrdeutiger Kontenerkennung `account_id` setzen.

```sh
noridrive --headless --config /absolut/profil.toml --dry-run
noridrive --headless --config /absolut/profil.toml --once --language de
```

Headless muss der absolute lokale Ordner bereits existieren. Browser-Profile verwenden und erneuern die zuvor in der Desktop-App eingerichtete Sitzung ohne Browserstart. App-Passwort-Profile verwenden den OS-Schlüsselspeicher oder `NORIDRIVE_PASSWORD` aus einem Secret Manager; `NORITEC_DRIVE_PASSWORD` bleibt kompatibel. Keine Passwörter in TOML oder Kommandozeilenargumente schreiben. `--dry-run` kann interne Metadaten anlegen, verändert aber keine Nutzdateien oder Remote-Objekte. `--once` wartet zunächst auf stabile Dateien und liefert nach zweimal der eingestellten Wartezeit plus 30 Sekunden einen Fehler, falls sie weiterhin gesperrt oder in Änderung sind. Die Desktop-App versucht es weiter. Headless abonniert keine Desktop-Energieereignisse.

## Grenzen und Wiederherstellung

Noch offen sind Dateien bei Bedarf, Explorer-Platzhalter, Blockübertragungen, Remote-Push, selektive Synchronisation, Mehrkonten-Oberfläche, Konflikt-/Wiederherstellungsdialoge, signierte Releases und automatische Updates. Übertragungen laufen seriell. Standardgrenzen: 1 GiB je Datei, 100.000 Einträge, 200 UTF-8-Bytes je Name und 240 je relativem Pfad. Symlinks/Reparse Points, unsichere Namen und Namenskollisionen stoppen den Scan. Rechte, Eigentümer, ACLs, alternative Datenströme und Zeitstempel werden nicht synchronisiert. Reine Änderungen der Groß-/Kleinschreibung sind noch nicht zuverlässig unterstützt.

Der komplette Namensraum wird weiterhin eingelesen; der Cache spart Inhaltszugriffe. Dateisperren und eine Wartezeit reduzieren Rennen, garantieren aber nicht für jedes Programm das Ende eines Schreibvorgangs. Unix-Sperren sind kooperativ. Bandbreitengrenzen betreffen Nutzdaten, nicht TLS-/HTTP-Overhead und Socket-Puffer. Weitere Grenzen stehen in der [Architektur](docs/architecture.md).

`.noritec-drive` nicht zur Reparatur löschen: Hier liegen Baseline, Journal, Teilübertragungen und Wiederherstellungskopien. Vor dem Zurückkopieren alter Fassungen den Abgleich pausieren. Lokale Kopien liegen in `.noritec-drive/history`, Remote-Kopien in `.noritec-drive-trash`, erreichbar etwa über Bulwark. Die Ablagen haben keine automatische Aufbewahrungsfrist und verbrauchen Speicher/Quota. Unabhängige Backups bleiben nötig.

Adapter berücksichtigen Stalwart 0.16 `nodeType`/`type` und das neuere `@type`/`mediaType`-Schema. Lokale HTTP-Tests prüfen beide; Live-Stalwart-Abnahme, Stromausfälle und große Bestände auf mehreren Geräten stehen weiterhin aus.
