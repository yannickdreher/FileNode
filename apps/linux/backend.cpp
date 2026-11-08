#include "backend.h"
#include <QApplication>
#include <QDateTime>
#include <QDesktopServices>
#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonArray>
#include <QLocale>
#include <QQuickStyle>
#include <QRegularExpression>
#include <QTimer>
#include <QUrl>
#include <QFileInfo>
#include <QScreen>

Backend::Backend(QString core, QString profile, bool noTray, bool background, QObject *parent)
    : QObject(parent), m_background(background) {
    QFile english(":/locales/en.json"), german(":/locales/de.json");
    // Resource aliases are stable across source and installed builds.
    if (!english.open(QIODevice::ReadOnly)) { english.setFileName(":/locales/en.json"); english.open(QIODevice::ReadOnly); }
    if (!german.open(QIODevice::ReadOnly)) { german.setFileName(":/locales/de.json"); german.open(QIODevice::ReadOnly); }
    m_en = QJsonDocument::fromJson(english.readAll()).object(); m_de = QJsonDocument::fromJson(german.readAll()).object();
    m_trayAvailable = !noTray && QSystemTrayIcon::isSystemTrayAvailable();
    m_show = m_menu.addAction("", this, &Backend::showStatus);
    m_settings = m_menu.addAction("", this, &Backend::showSettings); m_menu.addSeparator();
    m_toggle = m_menu.addAction("", this, [this] { m_running ? pause() : start(); });
    m_check = m_menu.addAction("", this, &Backend::check); m_open = m_menu.addAction("", this, &Backend::openFolder);
    m_menu.addSeparator(); m_quit = m_menu.addAction("", this, &Backend::quit);
    m_tray.setIcon(QIcon(":/filenode.svg")); m_tray.setContextMenu(&m_menu);
    connect(&m_tray, &QSystemTrayIcon::activated, this, [this](QSystemTrayIcon::ActivationReason reason) { if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick) emit showStatus(); });
    if (m_trayAvailable) m_tray.show();
    connect(&m_power, &PowerMonitor::changed, this, &Backend::publishPower);
    connect(&m_core, &QProcess::readyReadStandardOutput, this, &Backend::receive);
    connect(&m_core, &QProcess::errorOccurred, this, [this] { m_error = m_core.errorString(); emit changed(); });
    connect(&m_core, &QProcess::finished, this, [this](int code, QProcess::ExitStatus) {
        m_ready = false; if (!m_quitting) m_account = "failed";
        m_running = false; if (!m_quitting && code != 0) m_error = QString("Rust service exited with code %1").arg(code); emit changed();
        if (m_quitting) qApp->quit();
    });
    m_core.setProgram(core);
    QStringList arguments {"--service", "--gui-executable", QCoreApplication::applicationFilePath()};
    if (!profile.isEmpty()) arguments << "--config" << profile;
    m_core.setArguments(arguments); m_core.start(); updateTray();
}
Backend::~Backend() {
    if (m_core.state() != QProcess::NotRunning) { send({{"command","quit"}}); if (!m_core.waitForFinished(5000)) { m_core.kill(); m_core.waitForFinished(); } }
}
QString Backend::language() const {
    auto value = m_config.value("language", "auto").toString();
    return value == "auto" ? (QLocale::system().language() == QLocale::German ? "de" : "en") : value;
}
QString Backend::translate(const QString &key) const { return language() == "de" ? m_de.value(key).toString(m_en.value(key).toString(key)) : m_en.value(key).toString(key); }
QString Backend::status() const { return translate(m_status); }
QString Backend::style() const { return QQuickStyle::name(); }
QString Backend::version() const { return QCoreApplication::applicationVersion(); }
QString Backend::formatMessage(const QVariantMap &message) const {
    auto source = translate(message.value("key").toString());
    QVariantMap arguments;
    for (const auto &argument : message.value("arguments").toList()) { auto pair = argument.toList(); if (pair.size() == 2) arguments.insert(pair[0].toString(), pair[1]); }
    QRegularExpression expression("\\{([^{}]+)\\}");
    auto matches = expression.globalMatch(source); QString output; int offset = 0;
    while (matches.hasNext()) { auto match = matches.next(); output += source.mid(offset, match.capturedStart() - offset); output += arguments.value(match.captured(1), match.captured()).toString(); offset = match.capturedEnd(); }
    return output + source.mid(offset);
}
void Backend::send(QJsonObject command) {
    if (m_core.state() == QProcess::Running) m_core.write(QJsonDocument(command).toJson(QJsonDocument::Compact) + '\n');
}
void Backend::receive() {
    m_buffer += m_core.readAllStandardOutput();
    if (m_buffer.size() > 16 * 1024 * 1024) { m_error = "Desktop response exceeded limit"; m_core.kill(); emit changed(); return; }
    while (m_buffer.contains('\n')) {
        auto end = m_buffer.indexOf('\n'); auto event = QJsonDocument::fromJson(m_buffer.left(end)).object(); m_buffer.remove(0, end + 1);
        const auto type = event.value("event").toString();
        if (type == "configured") m_error.clear();
        if (type == "ready" || type == "configured" || event.contains("config")) { m_config = event.value("config").toObject().toVariantMap(); emit configChanged(); }
        if (type == "configured" && m_connectAfterSave) {
            m_connectAfterSave = false;
            send({{"command","connect"},{"password",m_pendingPassword},{"interactive",m_pendingInteractive}});
            m_pendingInteractive = false;
            m_pendingPassword.fill(QChar(0)); m_pendingPassword.clear();
        }
        if (type == "ready") {
            if (event.value("protocol").toInt() != 4) { m_error = "Incompatible Rust worker protocol"; emit changed(); continue; }
            m_ready = true; publishPower();
            if (!m_config.value("username").toString().isEmpty()) start();
            if (m_background && m_trayAvailable && !m_config.value("username").toString().isEmpty()) emit hideStatus();
        } else if (type == "open_browser") {
            const QUrl url(event.value("url").toString());
            if (url.scheme() == "https" && url.userInfo().isEmpty() && !QDesktopServices::openUrl(url)) { m_error = "The system browser could not open"; pause(); }
        } else if (type == "running") { m_running = event.value("value").toBool(); m_status = m_running ? "status.connecting" : "status.paused"; }
        else if (type == "status") m_status = event.value("key").toString();
        else if (type == "report") {
            auto report = event.value("report").toObject(); m_error.clear();
            m_status = report.value("failed_folders").toInt() > 0 ? "status.folder_error" : report.value("deferred").toInt() > 0 ? "status.deferred" : report.value("blocked").toInt() > 0 ? "status.deletions_blocked" : "status.current";
        } else if (type == "account") {
            m_account = event.value("state").toString();
            if (event.contains("trash_url")) m_trashUrl = event.value("trash_url").toString();
            if (m_account == "signed_out") { m_activity.clear(); m_folders.clear(); m_trashUrl.clear(); }
            if (m_account == "failed") m_activity.requireSignIn();
        } else if (type == "folder") { m_folders[event.value("folder").toString()] = event.toVariantMap(); if (event.value("state") == "current") m_activity.verifiedFolder(event.value("folder").toString()); }
        else if (type == "renamed") m_activity.renamed(event.value("folder").toString(), event.value("from").toString(), event.value("to").toString());
        else if (type == "file") {
            m_activity.upsert(event.toVariantMap());
            if (event.value("state") == "transferring") m_folders[event.value("folder").toString()] = QVariantMap{{"state","syncing"}};
        } else if (type == "error" || type == "retry") {
            if (m_account == "checking") m_account = "failed";
            m_connectAfterSave = false; m_pendingPassword.fill(QChar(0)); m_pendingPassword.clear(); m_error = event.value("details").toString(); m_status = "status.retry";
        } else if (type == "power") { m_powerPaused = event.value("paused").toBool(); if (m_powerPaused) m_status = "status.power_paused"; }
        updateTray(); emit changed();
    }
}
void Backend::apply(const QVariantMap &values, bool connectAccount, QString password, bool interactive) {
    auto next = m_config; for (auto item = values.begin(); item != values.end(); ++item) next.insert(item.key(), item.value());
    m_connectAfterSave = connectAccount;
    m_pendingInteractive = interactive;
    m_pendingPassword = password;
    send({{"command","configure"},{"config",QJsonObject::fromVariantMap(next)}});
    password.fill(QChar(0));
}
void Backend::start() { send({{"command","connect"}}); }
QString Backend::localPath(const QUrl &url) const { return url.toLocalFile(); }
void Backend::pause() { send({{"command","pause"}}); }
void Backend::check() { send({{"command","check"}}); }
void Backend::openPath(const QString &path) { if (!path.isEmpty()) QDesktopServices::openUrl(QUrl::fromLocalFile(path)); }
void Backend::openFolder() {
    const auto folders = m_config.value("sync_folders").toList();
    if (folders.size() == 1) openPath(folders.first().toMap().value("local_folder").toString());
    else emit showFolders();
}
void Backend::openOnline() {
    QUrl url(m_config.value("web_url").toString());
    if (url.scheme() == "https" && url.userInfo().isEmpty()) QDesktopServices::openUrl(url);
}
void Backend::openTrash() {
    const QUrl url(m_trashUrl.isEmpty() ? m_config.value("web_url").toString() : m_trashUrl);
    if (url.scheme() == "https" && url.userInfo().isEmpty()) QDesktopServices::openUrl(url);
}
QString Backend::folderStatus(const QString &folder) const {
    const auto state = !authenticated() ? "not_connected" : !m_running ? "paused" : m_powerPaused ? "power_paused" : m_folders.value(folder).toMap().value("state", "checking").toString();
    return translate("folder." + QString(state));
}
void Backend::quit() { m_quitting = true; send({{"command","quit"}}); if (m_core.state() == QProcess::NotRunning) qApp->quit(); }
QPoint Backend::popupPosition(int width, int height) const {
    const auto tray = m_tray.geometry();
    auto *screen = QGuiApplication::screenAt(tray.center()); if (!screen) screen = QGuiApplication::primaryScreen();
    const auto area = screen->availableGeometry();
    const auto x = tray.isEmpty() ? area.right() - width - 12 : tray.right() - width;
    const auto y = tray.isEmpty() ? area.bottom() - height - 12 : tray.center().y() < area.center().y() ? tray.bottom() + 8 : tray.top() - height - 8;
    return QPoint(qBound(area.left(), x, qMax(area.left(), area.right() - width)), qBound(area.top(), y, qMax(area.top(), area.bottom() - height)));
}
void Backend::publishPower() { send({{"command","power"},{"saver",m_power.saver},{"sleeping",m_power.sleeping},{"available",m_power.available}}); emit changed(); }
void Backend::injectPower(bool saver, bool sleeping) { m_power.saver = saver; m_power.sleeping = sleeping; m_power.available = true; publishPower(); }
void Backend::updateTray() {
    m_show->setText(translate("tray.show")); m_settings->setText(translate("tray.settings"));
    m_toggle->setText(translate(m_running ? "action.pause" : "action.resume")); m_check->setText(translate("action.check")); m_check->setEnabled(m_running);
    m_open->setText(translate("action.open")); m_quit->setText(translate("tray.quit")); m_tray.setToolTip("FileNode — " + status());
}
