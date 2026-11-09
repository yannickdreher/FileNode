# FileNode

Deutsch · [English](README.md) · [Website](https://yannickdreher.github.io/FileNode/) · [Downloads](https://github.com/yannickdreher/FileNode/releases)

JMAP-Dateisynchronisation für Windows und Linux mit gemeinsamem Rust-Kern. Windows verwendet ein eigenes **WinUI-3-/C#-Projekt**, Linux ein getrenntes **Qt-6-/C++-/QML-Projekt**. Die Oberfläche ist Deutsch/Englisch, Code und technische Diagnosen sind Englisch. Die Versionen 0.x sind Vorabversionen.

## Hintergrundbetrieb

Die App läuft im angemeldeten Benutzerkontext mit Tray-Icon, kompakter Aktivitätsansicht und eigenem Einstellungsfenster. Geschlossene Fenster beenden den Abgleich nicht. Erst Beenden stoppt die Anwendung und den Rust-Worker. Ein Windows-Systemdienst oder eine systemd-Unit wird nicht installiert. Unter Linux ist ein StatusNotifier-Host nötig, unter GNOME gegebenenfalls eine AppIndicator-Erweiterung. Ohne Tray das Statusfenster geöffnet oder minimiert lassen.

Beide Oberflächen teilen Übersetzungen, Profilformat und sämtliche Sync-Regeln. Unter KDE nutzt Qt bei vorhandenen Integrationspaketen den KDE-Desktop-Stil. Unter GNOME ist die Linux-Oberfläche weiterhin Qt und verwendet keine GTK-/libadwaita-Steuerelemente.

## Funktionen

- Bestätigte Anmeldung getrennt vom Sync-Status jedes Ordners. Pro Datei eine Aktivitätszeile mit echtem Übertragungsfortschritt sowie Erfolgs-/Wiederholungsstatus.
- Mehrere lokale Ordner pro Konto, Serverordner mit dem Namen der jeweiligen Auswahl und getrennte Zustände/Wiederholungen je Ordner.
- Vollständige lokale Dateien, bidirektionaler Abgleich, SHA-256-Inhaltsvergleich, SQLite-Dreiwege-Abgleich und dauerhaftes Journal.
- Bei Konflikten bleiben beide Fassungen erhalten. Remote-Umbenennungen nutzen stabile IDs; eindeutige lokale Dateiumbenennungen erhalten die Server-Identität.
- Lokale Löschungen verschieben Dateien und ganze Ordnerbäume automatisch in den gemeinsamen Server-Papierkorb, auch bei großen Mengen und kompatibel mit Bulwark. In Gegenrichtung schützen Mengenbegrenzungen vor massenhaftem Entfernen lokaler Arbeitskopien. Kein `FileNode/destroy`. Siehe [Papierkorb-Verhalten](docs/deletions.md).
- Native Dateisystem-Ereignisse unter Windows und Linux, Bündelung von Änderungen, Hash-Cache, regelmäßige vollständige Kontrolle und JMAP `FileNode/changes`.
- Bearbeitbare Ausschlüsse für temporäre Dateien, Office-Sperrdateien und unvollständige Downloads, in beide Richtungen ohne Löschung vorhandener Kopien. [Regeln](docs/exclusions.md).
- Vom Server ermittelte Uploadgrenzen, optionales eigenes Dateilimit und 1–16 parallele Blob-Uploads innerhalb der Servergrenze. Zu große Dateien erhalten eine eigene Warnung; andere Dateien laufen weiter. [Übertragungen](docs/transfers.md).
- Optionaler JMAP-WebSocket-Push mit Polling-Fallback und erneutem Abgleich nach Verbindungsabbrüchen. Downloads werden fortgesetzt; unterbrochene Uploads beginnen erneut vollständig.
- Native Diagnose im Windows-Anwendungsereignisprotokoll und Linux-Journal/Syslog sowie auswählbare Fehlerdetails je Ordner. [Diagnose](docs/diagnostics.md).
- Upload und Download getrennt in KiB/s begrenzen, auch während des Betriebs. Null bedeutet unbegrenzt.
- Gesperrte Dateien werden zurückgestellt; geänderte Dateien müssen über mehrere Beobachtungen stabil bleiben. Andere oberste Dateigruppen können weiter synchronisieren.
- Desktop-Energieereignisse pausieren Scans und Übertragungen bei Energiesparen/Standby. Die Energiesparoption ist abschaltbar; nach dem Aufwachen wird neu geprüft. Die App verhindert den Standby nicht.
- App-Passwörter im Windows Credential Manager oder Linux Secret Service, HTTPS und Prüfung der Endpoint-Origin.

## Installation unter Windows

**Installer (ab 0.2.2):** `filenode-X.Y.Z-windows-x86_64-setup.exe` unter [Releases](https://github.com/yannickdreher/FileNode/releases) herunterladen und starten. Er installiert für den aktuellen Benutzer unter `%LOCALAPPDATA%\Programs\FileNode`, legt einen Startmenü-Eintrag und eine Deinstallation an und verlangt weder Administratorrechte noch einen Zertifikatimport. Beim Upgrade von 0.2.4 oder älter FileNode einmal über das Tray-Menü beenden. Korrigierte Builds lassen Oberfläche und Sync-Prozess vom Installer automatisch beenden. Der Installer ist Deutsch/Englisch und folgt der hellen beziehungsweise dunklen Windows-Darstellung. Den Autostart kannst du anschließend in FileNodes allgemeinen Einstellungen aktivieren.

Die Setup-EXE ist derzeit unsigniert. Windows kann deshalb einen Hinweis auf den unbekannten Herausgeber oder SmartScreen anzeigen; verwaltete Geräte können unsignierte Programme einschränken. Der Installer ändert keine Windows-Sicherheitseinstellungen. Die Deinstallation entfernt die installierten Programmdateien und den zugehörigen Autostart-Eintrag. Konfiguration, gespeicherte Zugangsdaten und synchronisierte Ordner bleiben erhalten.

**Portables ZIP:** Das Windows-ZIP vollständig in einen dauerhaften Ordner entpacken und `filenode-ui.exe` starten. Dafür ist keine Installation nötig. Installer und ZIP unterstützen Windows 10 ab Build 19041 sowie Windows 11.

**MSIX:** Das angehängte MSIX ist für eine vertrauenswürdige Signierung oder die spätere Store-Einreichung vorgesehen. Unsigniert lässt es sich nicht direkt installieren. Die mit 0.2.1 veröffentlichte `-AllowUnsigned`-Anleitung war für FileNode falsch: Windows blockiert die ausführbare Aktivierung dieser eigenständigen WinUI-App mit `0x80073D2B`, auch mit der besonderen Publisher-OID und Administratorrechten. Der Weg für unsignierte Hosted Apps erlaubt keine eigene Aktivierung über `Executable`/`EntryPoint`. Siehe [Microsofts Hosted-App-Einschränkungen](https://learn.microsoft.com/en-us/windows/uwp/launch-resume/hosted-apps#requirements-for-unsigned-hosted-apps) und [MSIX-Signierung](https://learn.microsoft.com/en-us/windows/msix/package/signing-package-overview).

## Bauen und starten

Die [deutsche Entwicklungsanleitung](docs/development.de.md) beschreibt VS Code und Voraussetzungen. [Projektstruktur und Paketierung](docs/desktop-frontends.md) sind Englisch dokumentiert.

Windows mit Rust MSVC, C++ Build Tools, .NET 10 und Windows SDK:

```powershell
powershell.exe -NoProfile -File scripts/build-windows.ps1
& apps/windows/bin/x64/Debug/net10.0-windows10.0.26100.0/win-x64/filenode-ui.exe
# Unsigniertes lokales MSIX erstellen, ohne Installation oder Veröffentlichung:
powershell.exe -NoProfile -File scripts/build-windows.ps1 -Configuration Release -Package
```

Linux mit Qt 6.4+ samt Entwicklungs- und QML-Laufzeitmodulen:

```sh
sh scripts/build-linux.sh
./target/debug/filenode-ui
```

Pull Requests führen Linux- und native Windows-Prüfungen in GitHub Actions aus. Nach einem Merge in `main` plant semantic-release die Version, baut beide Plattformen und veröffentlicht einen GitHub-Release mit Änderungsnotizen, Downloads und Prüfsummen. Die bereitgestellten Windows-Runner benötigen keine eigene Runner-Installation. GitHub Pages veröffentlicht die deutsch/englische Website. Siehe [Releases](docs/releases.md).

## Verbinden

Unter **Synchronisation** die lokalen Ordner hinzufügen. Unter **Konto** Server und Benutzername eingeben. Vorgaben sind `https://jmap.example.com` für JMAP und `https://mail.example.com` für den Webclient. Als Anmeldemethode **Browser / Single Sign-on** für OAuth Authorization Code mit PKCE oder **App-Passwort** auswählen und anschließend **Anmelden** drücken. Vor dem Verbinden eines Testordners die Einstellung zur Löschweitergabe prüfen. Die [Browser-Anmeldedokumentation](docs/browser-authentication.md) beschreibt Server-Voraussetzungen und externe OIDC-Anbieter.

**Anmelden** speichert die Eingaben vor der Authentifizierung. Serverordner verwenden den letzten Namen der Auswahl: `C:\Users\Yannick\Desktop` wird zu `/Desktop`, `C:\Users\Yannick\Dokumente\Steuern` zu `/Steuern`. Überlappende lokale oder serverseitige Auswahlen werden abgelehnt. Ordner und Synchronisationseinstellungen können während des Abgleichs geändert werden. Gespeicherte Änderungen starten den Worker bei Bedarf sicher neu; der Abgleich läuft automatisch weiter. Nach dem Hinzufügen beginnt der Abgleich automatisch, sofern das Konto angemeldet ist. **Synchronisation beenden** erhält lokale und serverseitige Dateien. Siehe [Ordnerauswahl](docs/folder-selection.md).


Ein eingerichtetes Konto setzt die Synchronisation beim Start von FileNode fort. Dafür gibt es keine separate Startoption. Die Browser-Authentifizierung verwendet gespeicherte Tokens; eine interaktive Browser-Anmeldung wird weiterhin nur über die Anmeldeaktion gestartet.

Passwortfreie Profile liegen unter Windows in `%APPDATA%\FileNode\config\config.toml`, unter Linux in `$XDG_CONFIG_HOME/filenode/config.toml` beziehungsweise `~/.config/filenode/config.toml`. `--config <Pfad>` wählt ein anderes Profil. Optionen stehen in [config.example.toml](config.example.toml). Bei mehrdeutiger Kontenerkennung `account_id` setzen.

```sh
filenode --headless --config /absolut/profil.toml --dry-run
filenode --headless --config /absolut/profil.toml --once --language de
```

Headless werden absolute Ordnerpfade benötigt. Browser-Profile verwenden und erneuern die zuvor in der Desktop-App eingerichtete Sitzung ohne Browserstart. App-Passwort-Profile verwenden den OS-Schlüsselspeicher oder `FILENODE_PASSWORD` aus einem Secret Manager. Keine Passwörter in TOML oder Kommandozeilenargumente schreiben. `--dry-run` liefert einen Plan pro Auswahl und kann interne Metadaten anlegen, verändert aber keine Nutzdateien oder Remote-Objekte. `--once` meldet einen Fehler, sobald ein ausgewählter Ordner fehlschlägt. `--once` wartet zunächst auf stabile Dateien und liefert nach zweimal der eingestellten Wartezeit plus 30 Sekunden einen Fehler, falls sie weiterhin gesperrt oder in Änderung sind. Die Desktop-App versucht es weiter. Headless abonniert keine Desktop-Energieereignisse.

## Grenzen und Wiederherstellung

Noch offen sind Dateien bei Bedarf, Explorer-Platzhalter, Blockübertragungen, Upload-Fortsetzung ab beliebiger Byteposition, selektive Synchronisation, Mehrkonten-Oberfläche, Konflikt-/Wiederherstellungsdialoge, signierte Releases und automatische Updates. Blob-Uploads können parallel laufen; Downloads und Änderungen an FileNodes bleiben seriell. Automatisch gilt das Uploadlimit des Servers, bei fehlender Angabe ersatzweise 1 GiB. Weitere Grenzen: 100.000 Einträge, 200 UTF-8-Bytes je Name und 240 je relativem Pfad. Symlinks/Reparse Points, unsichere Namen und Namenskollisionen stoppen den Scan. Rechte, Eigentümer, ACLs, alternative Datenströme und Zeitstempel werden nicht synchronisiert. Reine Änderungen der Groß-/Kleinschreibung sind noch nicht zuverlässig unterstützt.

Der komplette Namensraum wird weiterhin eingelesen; der Cache spart Inhaltszugriffe. Dateisperren und eine Wartezeit reduzieren Rennen, garantieren aber nicht für jedes Programm das Ende eines Schreibvorgangs. Unix-Sperren sind kooperativ. Bandbreitengrenzen betreffen Nutzdaten, nicht TLS-/HTTP-Overhead und Socket-Puffer. Weitere Grenzen stehen in der [Architektur](docs/architecture.md).

`.filenode` nicht zur Reparatur löschen: Hier liegen Baseline, Journal, Teilübertragungen und Wiederherstellungskopien. Vor dem Zurückkopieren alter Fassungen den Abgleich pausieren. Der Papierkorb verwendet den Server-Papierkorb. Davon getrennte interne Kopien alter Fassungen liegen lokal in `.filenode/history` und remote in `.filenode-history`. Die Ablagen haben keine automatische Aufbewahrungsfrist und verbrauchen Speicher/Quota. Unabhängige Backups bleiben nötig.

Adapter berücksichtigen Stalwart 0.16 `nodeType`/`type` und das neuere `@type`/`mediaType`-Schema. Lokale HTTP-Tests prüfen beide; Live-Stalwart-Abnahme, Stromausfälle und große Bestände auf mehreren Geräten stehen weiterhin aus.
