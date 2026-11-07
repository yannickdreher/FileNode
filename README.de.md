# Noritec Drive

Deutsch · [English](README.md)

Ein Rust-Desktop-Client für **Stalwart JMAP Files**, mit Builds für Windows und Linux und einer deutsch/englischen Oberfläche. **Version 0.1 ist eine Preview** mit vollständigen lokalen Dateien und automatisierten Fehlertests. Die langfristige Zuverlässigkeit im Produktivbetrieb ist noch nicht nachgewiesen.

## Funktionen

Die App läuft über ein Symbol im Infobereich. Ein Klick öffnet eine kompakte Statusansicht mit letzten Dateiaktivitäten und Schnellzugriffen; die Einstellungen erscheinen in einem eigenen Fenster. Das Schließen eines Fensters lässt den Abgleich weiterlaufen. Pause/Fortsetzen und Beenden sind über das Tray- und Statusmenü erreichbar. Unter Allgemein kann der Start bei der Betriebssystem-Anmeldung aktiviert werden. Ohne nutzbaren Tray bleibt das Statusfenster sichtbar.

- Dateien und Ordner werden in beide Richtungen mit einem eigenen Remote-Ordner `Noritec Drive` abgeglichen. Andere oberste Ordner in Bulwark gehören nicht zu diesem Profil.
- SHA-256-Inhaltsvergleich, persistenter Dreiwege-Abgleich in SQLite und Erkennung gleichzeitiger Änderungen. Bei Konflikten bleiben beide Fassungen erhalten: Die Serverfassung erhält den ursprünglichen Pfad, die lokale Fassung einen zusätzlichen Namen `name (conflict <id>).ext` und wird im nächsten Durchlauf hochgeladen.
- Remote-Umbenennungen anhand stabiler IDs und eindeutige lokale Dateiumbenennungen anhand unveränderter Inhalte. Komplexere Fälle nutzen Übertragung und Wiederherstellungsablage.
- Löschungen werden erst nach ausdrücklicher Aktivierung weitergegeben. Lokal entfernte Einträge werden auf dem Server in `.noritec-drive-trash` verschoben; serverseitig entfernte Einträge lokal nach `.noritec-drive/history/<id>/`. Vor dem Überschreiben auf dem Server entsteht ebenfalls eine Wiederherstellungskopie. Der Client ruft `FileNode/destroy` nicht auf.
- Schutz vor Massenlöschungen, unvollständigen Scans, verschwundenen Remote-Sync-Ordnern, unsicheren Pfaden und konkurrierenden Änderungen. Ein Prozess pro lokalem Ordner, dauerhaftes Journal, erneute Bestandsaufnahme nach unklaren Antworten, automatische Wiederholungen in der GUI und fortsetzbare Downloads.
- Zugangsdaten im Windows Credential Manager beziehungsweise Linux Secret Service, ausschließlich HTTPS und keine Weitergabe der Zugangsdaten an andere Origins.
- Native Dateisystem-Ereignisse, Bündelung kurzer Änderungsserien, Metadaten-/Hash-Cache, regelmäßige vollständige Kontrollprüfung und Wiederverwendung der JMAP-Verbindung. Unveränderte Baselines werden nicht ständig neu in SQLite geschrieben.
- Systemsprache automatisch erkennen oder **Deutsch/English** direkt umschalten. Die Auswahl wird pro Profil gespeichert. Auch vorhandene Aktivitätsmeldungen wechseln die Sprache. Code, technische Diagnosen, Protokollwerte und Konfliktdateinamen bleiben Englisch.

## Starten

Das portable Archiv aus den Artefakten des Jobs **test-and-package** einer erfolgreichen GitLab-Pipeline herunterladen und entpacken. Unter Windows `noritec-drive.exe`, unter Linux `./noritec-drive` starten. Windows-Builds sind derzeit nicht signiert. Der Linux-Build setzt x86-64, Debian 12/glibc 2.36 oder neuer, X11/Wayland, OpenGL, D-Bus, für X11 zusätzlich `libxkbcommon-x11-0` und einen entsperrten Secret Service voraus, beispielsweise GNOME Keyring oder entsprechend eingerichtetes KDE Wallet.

1. Unter **Einstellungen → Konto** Server, E-Mail/Benutzername und ein **Stalwart-App-Passwort** eintragen. Vorgabe ist `https://jmap.example.com`.
2. Unter **Synchronisation und Wiederherstellung** einen eigenen lokalen Ordner wählen; er wird bei Bedarf angelegt. Pro Server/Konto/Profil einen getrennten Ordner verwenden.
3. Die Weitergabe von Löschungen zunächst ausgeschaltet lassen und **Verbinden & synchronisieren** wählen. Unterschiedliche Inhalte am selben Pfad werden beim ersten Abgleich als Konflikt erhalten.
4. Das Ergebnis prüfen und bei Bedarf im pausierten Zustand die Weitergabe von Löschungen einschalten. Genügend Platz für alle Dateien und Wiederherstellungskopien einplanen.

Das Dateikonto wird per JMAP ermittelt. Falls mehrere Konten infrage kommen, `account_id` im Profil festlegen. Die Anmeldung nutzt derzeit App-Passwörter über HTTPS; ein interaktiver authentik-/SSO-Browser-Login ist noch nicht enthalten.

## Profile und Kommandozeile

Die GUI speichert eine passwortfreie `config.toml` im Benutzer-Konfigurationsverzeichnis: unter Windows `%APPDATA%\Noritec\Drive\config\`, unter Linux `$XDG_CONFIG_HOME/drive/` beziehungsweise `~/.config/drive/`. Über `--config <Pfad>` sind weitere Profile möglich. [config.example.toml](config.example.toml) zeigt die Optionen; für den Headless-Betrieb muss der lokale Ordner als absoluter Pfad bereits existieren.

```sh
noritec-drive --language de
noritec-drive --language en
noritec-drive --headless --config /pfad/profil.toml --dry-run
noritec-drive --headless --config /pfad/profil.toml --once --language de
```

Headless wird das App-Passwort aus dem Betriebssystem-Schlüsselspeicher gelesen; alternativ kann ein Secret Manager `NORITEC_DRIVE_PASSWORD` bereitstellen. Passwort weder in TOML noch als Kommandozeilenargument ablegen. Ein Build ohne Desktop-Features benötigt diese Umgebungsvariable. CLI-Hilfe, JSON-Pläne und technische Fehlerdetails sind Englisch; verständliche Abgleichmeldungen folgen der gewählten Sprache.

Windows-Desktop-Releases öffnen kein zusätzliches Konsolenfenster. In PowerShell-Skripten ausdrücklich auf den Prozess warten (`Start-Process -Wait`, bei Bedarf mit Ausgabeumleitung). Interaktive GUI-Programme blockieren die aufrufende Shell nicht immer. Native Debug-Builds behalten ihre Konsole für den Debugger.

`--dry-run` verändert keine Nutzdateien oder Remote-Objekte, kann aber lokale Profilmetadaten anlegen. `--once` erledigt notwendige Folgeprüfungen nach Konflikten/Umbenennungen. Headless beendet sich bei Netzwerkfehlern mit einem Fehlercode; ein Service-Manager kann den Prozess erneut starten. Die GUI versucht es automatisch erneut.

## Wiederherstellung und Grenzen

Lokale Sicherungsfassungen befinden sich in `.noritec-drive/history`; die gewünschte Fassung zurück in den Sync-Ordner kopieren. Remote-Sicherungen liegen in `.noritec-drive-trash` im Remote-Sync-Ordner und lassen sich etwa über Bulwark zurückkopieren oder verschieben. Vor einer manuellen Wiederherstellung den Abgleich stoppen. Die Ablagen werden nicht automatisch bereinigt und verbrauchen Speicherplatz/Quota. Sie ersetzen keine unabhängige Datensicherung; andere Clients können Dateien dauerhaft entfernen.

`.noritec-drive` nicht zur Fehlerbehebung löschen: Hier liegen Abgleichzustand, Journal, Teilübertragungen und lokale Sicherungsfassungen. Bei verschwundenem Remote-Sync-Ordner stoppt der Client absichtlich. Zunächst den Zustand auf dem Server prüfen und wiederherstellen.

Lokale Dateisystem-Ereignisse lösen nach 600 ms Ruhezeit einen Abgleich aus; spätestens nach 2 Sekunden wird eine Änderungsserie geprüft. Der komplette lokale Namensraum wird weiterhin vor der Planung eingelesen. Unveränderte Fingerprints verwenden zwischengespeicherte Hashes. Geänderte Pfade, verlorene Ereignisse, Dateiwächterfehler, erneutes Verbinden, manuelle Prüfungen und eine vollständige Kontrolle alle 15 Minuten verwerfen den Cache. Ohne funktionierenden Dateiwächter wird bei jedem Durchlauf vollständig gehasht. Remote werden standardmäßig alle 30 Sekunden nach dem ersten Snapshot `FileNode/changes` abgefragt. Die Desktop-Anwendung verwendet ihre HTTP-Verbindung weiter. Übertragungen laufen seriell; Uploads werden vollständig neu übertragen, Downloads können fortgesetzt werden. Grenzen: standardmäßig 1 GiB je Datei, maximal 100.000 Einträge, 200 UTF-8-Bytes je Name und 240 Bytes je relativem Pfad. Windows-reservierte Namen, Groß-/Kleinschreibungs- und Normalisierungskollisionen, Symlinks, Reparse Points und Spezialdateien stoppen den Scan. `.git` und die internen Ablagen sind ausgeschlossen. Berechtigungen, Eigentümer, ACLs, alternative Datenströme, Hardlink-Identität und Zeitstempel werden nicht synchronisiert. Reine Änderungen der Groß-/Kleinschreibung sind auf entsprechenden Dateisystemen noch nicht zuverlässig unterstützt.

**Noch offen:** Dateien bei Bedarf/Platzhalter wie bei OneDrive, Explorer-Integration, virtuelles Linux-Dateisystem, Browser-OAuth/SSO, Mehrkonten-Verwaltung, selektive Synchronisation, Rechte geteilter Ordner, Konflikt-/Wiederherstellungsdialoge, Bandbreitenbegrenzung, signierte Installer und automatische Updates. Tests gegen ein echtes Stalwart-Testkonto, Stromausfälle und große Bestände mit mehreren Geräten stehen noch aus. Zwischen letzter Prüfung und Umbenennung kann eine andere Anwendung eine Datei verändern; der Client bietet keine Dateisystemtransaktionen gegenüber anderen Programmen.

Die Protokolladapter berücksichtigen Stalwart **0.16 `nodeType`/`type`** und das neuere **`@type`/`mediaType`**-Schema. Beide werden mit echten HTTP-Anfragen gegen einen lokalen Testserver geprüft. FileNode ist noch ein Entwurf; die konkrete Serverversion muss vor dem Produktiveinsatz mit einem Testkonto abgenommen werden.

Entwicklung, Architektur und Build-Befehle stehen im [englischen README](README.md) und in der [Architekturbeschreibung](docs/architecture.md). Übersetzungen liegen in `locales/en.json` und `locales/de.json`; automatisierte Tests prüfen Schlüssel und Platzhalter.

## VS Code

Für Start und Debugging mit F5 den Repository-Ordner öffnen und Rust sowie die empfohlenen Erweiterungen rust-analyzer und CodeLLDB installieren. Unter Windows werden zusätzlich die C++ Build Tools und das Windows SDK benötigt. **Drive: desktop and settings** verwendet ein eigenes, von Git ignoriertes Debug-Profil ohne automatische Anmeldung oder Löschweitergabe. Details stehen in der [deutschen Debugging-Anleitung](docs/development.de.md).
