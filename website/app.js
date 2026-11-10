const messages = {
  en: {
    devEyebrow: "IN DEVELOPMENT · LINUX PREVIEW",
    devTitle: "At home on GNOME and KDE.",
    devIntro: "The development build adds native GNOME and KDE interfaces and Files On-Demand through FUSE, with Nautilus and Dolphin actions. Download buttons point to the latest published release; these Linux additions are still in development.",
    devRulesTitle: "Your folders, your rules.",
    devRulesText: "Choose which folders to sync, exclude files you don’t need and control upload speed. FileNode shows you when a file needs attention.",
    devConnectTitle: "Ready for changing connections.",
    devConnectText: "JMAP WebSocket notifications trigger remote checks when supported, with polling as a fallback. Downloads resume after interruptions; uploads restart the affected file from the beginning.",
    devDetailsTitle: "Know what is happening.",
    devDetailsText: "See separate sync states for every folder, actionable file-size warnings and selectable error details. Diagnose failures through Windows Event Log or Linux journal/syslog. Deletions use the shared server trash.",
    devLink: "Follow the 0.2 preview ↗",
    skip: 'Skip to content', preview: 'Preview', nav: 'Main navigation', language: 'Language', howNav: 'How it works', storeSoon: 'Microsoft Store — coming soon', storeOpen: 'Open Microsoft Store', storeWeb: 'View in browser ↗', storeHint: 'The Store listing is being prepared. Portable downloads are available through GitHub Releases.',
    eyebrow: 'YOUR CLOUD, A LITTLE CLOSER', headline: 'Your files.<br>At home on<br><em>your desktop.</em>',
    intro: 'Bring your JMAP files to Windows and Linux. FileNode keeps a local copy in sync while you get on with your day.',
    getApp: 'Get FileNode', explore: 'Explore the project ↗', heroMeta: 'Windows + Linux · English + Deutsch · Built with Rust',
    illustration: "Illustration of FileNode's tray activity panel", quiet: 'Quietly in the background.', upToDate: 'Everything is up to date', activity: 'RECENT ACTIVITY', syncedDesktop: 'Synced to Desktop', available: 'Available on this device', openFolder: 'Open folder', viewOnline: 'View online', ownServer: 'Your server. Your files.', artCaption: 'Illustrative activity preview',
    principle1: 'A real local copy', principle2: 'Your existing JMAP server', principle3: 'A place in your system tray',
    howEyebrow: 'LESS SETUP. MORE DOING.', howTitle: 'Pick a folder.<br>Make yourself at home.', howText: 'Connect your JMAP Files account, choose a local folder, and sign in. FileNode creates a matching folder on your server. No remote paths to figure out.', onDevice: 'ON YOUR DEVICE', onServer: 'ON YOUR SERVER', signInDocs: 'Browser sign-in & server requirements ↗',
    nativeTitle: 'Fits your desktop.', nativeText: "WinUI 3 on Windows. The Linux development build adds GTK/libadwaita on GNOME and Kirigami on KDE, sharing one Rust sync engine.", careTitle: 'A little extra care.', careText: 'Conflicting edits keep both versions. Busy files can wait. Deletion sync is your choice, with recovery storage for older copies.', paceTitle: 'At your own pace.', paceText: 'Set separate upload and download limits. Desktop power signals pause scanning and transfers during energy saving and sleep.',
    downloadEyebrow: 'YOUR NEXT QUIET LITTLE UTILITY', downloadTitle: 'Meet your new sync companion.', downloadIntro: 'FileNode is a preview. Start with a dedicated test folder and a compatible JMAP Files server.',
    windowsText: 'Native WinUI interface. Extract the portable ZIP and start FileNode.', windowsButton: 'Windows downloads', windowsMeta: 'x64 · Portable ZIP · Unsigned MSIX also available', linuxText: "Native GNOME and KDE interfaces are being prepared, with the shared sync core and CLI. Check release notes for included features.", linuxButton: 'Linux downloads', linuxMeta: "x86_64 · New development baseline: Debian 13", releaseFallback: 'Available builds and change notes are listed on GitHub Releases.', allReleases: 'All releases & checksums ↗', releaseAvailable: 'Latest release: ',
    faqTitle: 'A few things to know.', faq1Title: 'What server do I need?', faq1Text: 'A server that supports JMAP Files, such as a compatible Stalwart deployment. FileNode is a client for your server, not a hosted storage service. Browser sign-in depends on your server and identity provider configuration; app-password sign-in is also supported.', faq2Title: 'Are my files available offline?', faq2Text: "Full synchronization keeps ordinary local files. Files On-Demand uses Explorer on Windows and FUSE in the Linux development build. Pin files to cache them offline. Linux cloud folders remain mounted only while FileNode runs; cached contents are retained after quitting.", faq3Title: 'Does it work on GNOME?', faq3Text: "The Linux development build provides native GTK 4/libadwaita on GNOME and Qt 6/Kirigami on KDE. GNOME reopens FileNode from the application launcher; KDE uses its tray. Separate Nautilus and Dolphin extensions add availability and keep/free-space actions.", faq4Title: 'Is the Windows package signed?', faq4Text: 'The current MSIX is unsigned and uses a development identity. It needs signing or trust configuration for normal installation. The portable ZIP is a separate download. Microsoft Store publication is not available yet.', footer: 'A small place for your files to feel at home.', source: 'Source & documentation ↗', title: 'FileNode — Your files. At home on your desktop.'
  },
  de: {
    devEyebrow: "IN ENTWICKLUNG · LINUX-VORSCHAU",
    devTitle: "Zu Hause auf GNOME und KDE.",
    devIntro: "Der Entwicklungsstand ergänzt native GNOME- und KDE-Oberflächen und Dateien bei Bedarf über FUSE, mit Aktionen für Nautilus und Dolphin. Die Download-Buttons zeigen den zuletzt veröffentlichten Release; die Linux-Ergänzungen sind noch in Entwicklung.",
    devRulesTitle: "Deine Ordner, deine Regeln.",
    devRulesText: "Wähle deine Ordner aus, schließe unnötige Dateien aus und bestimme die Upload-Geschwindigkeit. FileNode zeigt dir, wenn eine Datei Aufmerksamkeit braucht.",
    devConnectTitle: "Auch wenn das Netz wechselt.",
    devConnectText: "JMAP-WebSocket-Benachrichtigungen stoßen bei Serverunterstützung einen Abgleich an, mit Polling als Fallback. Downloads werden nach Abbrüchen fortgesetzt; Uploads beginnen für die betroffene Datei von vorn.",
    devDetailsTitle: "Wissen, was gerade passiert.",
    devDetailsText: "Getrennte Zustände je Ordner, verständliche Dateigrößenwarnungen und auswählbare Fehlerdetails. Diagnosen im Windows-Ereignisprotokoll oder Linux-Journal/Syslog. Löschungen verwenden den gemeinsamen Server-Papierkorb.",
    devLink: "Die Vorschau 0.2 verfolgen ↗",
    skip: 'Zum Inhalt', preview: 'Vorschau', nav: 'Hauptnavigation', language: 'Sprache', howNav: 'So funktioniert’s', storeSoon: 'Microsoft Store — demnächst', storeOpen: 'Microsoft Store öffnen', storeWeb: 'Im Browser ansehen ↗', storeHint: 'Der Store-Eintrag wird vorbereitet. Portable Downloads findest du unter GitHub Releases.',
    eyebrow: 'DEINE CLOUD, EIN STÜCK NÄHER', headline: 'Deine Dateien.<br>Zu Hause auf<br><em>deinem Desktop.</em>',
    intro: 'Deine JMAP-Dateien auf Windows und Linux. FileNode hält eine lokale Kopie synchron, während du dich um deinen Alltag kümmerst.',
    getApp: 'FileNode entdecken', explore: 'Zum Projekt ↗', heroMeta: 'Windows + Linux · Deutsch + English · Entwickelt mit Rust',
    illustration: 'Illustration der FileNode-Aktivitätsansicht im Tray', quiet: 'Arbeitet leise im Hintergrund.', upToDate: 'Alles auf dem neuesten Stand', activity: 'LETZTE AKTIVITÄTEN', syncedDesktop: 'Mit Desktop synchronisiert', available: 'Auf diesem Gerät verfügbar', openFolder: 'Ordner öffnen', viewOnline: 'Online ansehen', ownServer: 'Dein Server. Deine Dateien.', artCaption: 'Illustrative Aktivitätsvorschau',
    principle1: 'Eine echte lokale Kopie', principle2: 'Dein vorhandener JMAP-Server', principle3: 'Ein Platz im System-Tray',
    howEyebrow: 'WENIGER EINRICHTEN. MEHR ERLEDIGEN.', howTitle: 'Ordner auswählen.<br>Zu Hause ankommen.', howText: 'Verbinde dein JMAP-Dateikonto, wähle einen lokalen Ordner und melde dich an. FileNode erstellt den passenden Ordner auf deinem Server. Ohne Remote-Pfade einzutragen.', onDevice: 'AUF DEINEM GERÄT', onServer: 'AUF DEINEM SERVER', signInDocs: 'Browser-Anmeldung & Server-Voraussetzungen ↗',
    nativeTitle: 'Passt auf deinen Desktop.', nativeText: "WinUI 3 unter Windows. Der Linux-Entwicklungsstand ergänzt GTK/libadwaita für GNOME und Kirigami für KDE, mit gemeinsamem Rust-Sync-Kern.", careTitle: 'Mit etwas mehr Sorgfalt.', careText: 'Bei Konflikten bleiben beide Versionen erhalten. Beschäftigte Dateien können warten. Die Löschweitergabe bestimmst du, mit Sicherungen älterer Fassungen.', paceTitle: 'In deinem Tempo.', paceText: 'Begrenze Upload und Download getrennt. Desktop-Energiesignale pausieren Scans und Übertragungen beim Energiesparen und im Standby.',
    downloadEyebrow: 'DEIN NÄCHSTER LEISER ALLTAGSHELFER', downloadTitle: 'Dein neuer Sync-Begleiter.', downloadIntro: 'FileNode ist eine Vorschauversion. Starte mit einem eigenen Testordner und einem kompatiblen JMAP-Dateiserver.',
    windowsText: 'Native WinUI-Oberfläche. Das portable ZIP entpacken und FileNode starten.', windowsButton: 'Windows-Downloads', windowsMeta: 'x64 · Portables ZIP · Auch als unsigniertes MSIX', linuxText: "Native GNOME- und KDE-Oberflächen mit gemeinsamem Sync-Kern und CLI sind in Vorbereitung. Die Release-Notizen nennen die enthaltenen Funktionen.", linuxButton: 'Linux-Downloads', linuxMeta: "x86_64 · Neue Entwicklungsbasis: Debian 13", releaseFallback: 'Verfügbare Builds und Änderungen findest du unter GitHub Releases.', allReleases: 'Alle Releases & Prüfsummen ↗', releaseAvailable: 'Aktueller Release: ',
    faqTitle: 'Noch gut zu wissen.', faq1Title: 'Welchen Server brauche ich?', faq1Text: 'Einen Server mit JMAP-Dateiunterstützung, etwa eine kompatible Stalwart-Installation. FileNode ist ein Client für deinen Server und kein gehosteter Speicherdienst. Die Browser-Anmeldung hängt von Server und Identitätsanbieter ab; die Anmeldung per App-Passwort ist ebenfalls möglich.', faq2Title: 'Sind meine Dateien offline verfügbar?', faq2Text: "Der vollständige Abgleich hält normale lokale Dateien bereit. Dateien bei Bedarf verwenden unter Windows den Explorer, im Linux-Entwicklungsstand FUSE. Markierte Dateien werden offline zwischengespeichert. Linux-Cloud-Ordner sind nur bei laufendem FileNode eingehängt; der Cache bleibt nach dem Beenden erhalten.", faq3Title: 'Funktioniert es unter GNOME?', faq3Text: "Der Linux-Entwicklungsstand bietet GTK 4/libadwaita für GNOME und Qt 6/Kirigami für KDE. Unter GNOME öffnest du FileNode über den Anwendungsstarter, KDE nutzt das Tray. Separate Nautilus- und Dolphin-Erweiterungen ergänzen Verfügbarkeit und Offline-/Freigabeaktionen.", faq4Title: 'Ist das Windows-Paket signiert?', faq4Text: 'Das aktuelle MSIX ist unsigniert und nutzt eine Entwicklungsidentität. Für eine normale Installation ist eine Signierung oder Vertrauenseinrichtung nötig. Das portable ZIP ist ein separater Download. Eine Veröffentlichung im Microsoft Store gibt es noch nicht.', footer: 'Ein kleines Zuhause für deine Dateien.', source: 'Quellcode & Dokumentation ↗', title: 'FileNode — Deine Dateien. Zu Hause auf deinem Desktop.'
  }
};

let currentLanguage = 'en';
let releaseTag = '';
function setLanguage(language) {
  currentLanguage = language === 'de' ? 'de' : 'en';
  const text = messages[currentLanguage];
  document.documentElement.lang = currentLanguage;
  document.title = text.title;
  document.querySelectorAll('[data-i18n]').forEach(element => { element.textContent = text[element.dataset.i18n]; });
  // These HTML strings are fixed translations in this file, never remote content.
  document.querySelectorAll('[data-i18n-html]').forEach(element => { element.innerHTML = text[element.dataset.i18nHtml]; });
  document.querySelectorAll('[data-i18n-aria]').forEach(element => { element.setAttribute('aria-label', text[element.dataset.i18nAria]); });
  document.querySelectorAll('[data-language]').forEach(button => { button.setAttribute('aria-pressed', String(button.dataset.language === currentLanguage)); });
  if (releaseTag) document.getElementById('release-status').textContent = text.releaseAvailable + releaseTag;
  try { localStorage.setItem('filenode-language', currentLanguage); } catch { /* Browser storage can be disabled. */ }
}

let preferred = navigator.language.startsWith('de') ? 'de' : 'en';
try { preferred = localStorage.getItem('filenode-language') || preferred; } catch { /* Keep browser preference. */ }
setLanguage(preferred);
document.querySelectorAll('[data-language]').forEach(button => button.addEventListener('click', () => setLanguage(button.dataset.language)));

const productId = window.FileNodeConfig?.microsoftStoreProductId || '';
if (/^[A-Z0-9]{12}$/.test(productId)) {
  const storePage = `https://apps.microsoft.com/detail/${productId}`;
  const isWindows = /Windows/i.test(navigator.userAgent);
  const storeLink = document.getElementById('store-link');
  storeLink.href = isWindows ? `ms-windows-store://pdp/?ProductId=${productId}` : storePage;
  storeLink.hidden = false;
  document.getElementById('store-soon').hidden = true;
  const webLink = document.getElementById('store-web-link');
  webLink.href = storePage;
  webLink.hidden = false;
}

async function loadRelease() {
  try {
    const response = await fetch('https://api.github.com/repos/yannickdreher/FileNode/releases/latest', { signal: AbortSignal.timeout(5000) });
    if (!response.ok) return;
    const release = await response.json();
    if (!Array.isArray(release.assets) || !/^v\d+\.\d+\.\d+$/.test(release.tag_name)) return;
    const patterns = { windows: /-windows-x86_64\.zip$/, linux: /-linux-x86_64\.tar\.gz$/ };
    for (const [platform, pattern] of Object.entries(patterns)) {
      const asset = release.assets.find(candidate => pattern.test(candidate.name));
      if (asset?.browser_download_url?.startsWith('https://github.com/yannickdreher/FileNode/releases/download/')) {
        document.getElementById(`${platform}-download`).href = asset.browser_download_url;
      }
    }
    releaseTag = release.tag_name;
    document.getElementById('release-status').textContent = messages[currentLanguage].releaseAvailable + releaseTag;
  } catch { /* Release-list links remain usable without API access. */ }
}
loadRelease();
