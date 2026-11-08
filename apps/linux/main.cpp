#include "backend.h"
#include <QApplication>
#include <QCommandLineParser>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQuickStyle>
#include <QLibraryInfo>
#include <QDir>
#include <QQuickWindow>
#include <QTimer>
#include <QFile>
#include <QIcon>
#include <QJsonDocument>

int main(int argc, char **argv) {
    QApplication application(argc, argv);
    application.setApplicationVersion(NORIDRIVE_VERSION);
    application.setApplicationName("NoriDrive"); application.setOrganizationName("Noritec"); application.setQuitOnLastWindowClosed(false);
    QCommandLineParser parser; parser.addHelpOption(); parser.addVersionOption();
    parser.addOptions({QCommandLineOption("core", "Rust service executable", "path"), QCommandLineOption("config", "Profile path", "path"), QCommandLineOption("background", "Start in tray"), QCommandLineOption("settings", "Open settings"), QCommandLineOption("no-tray", "Disable tray"), QCommandLineOption("self-test", "Offline UI smoke test and screenshot directory", "directory")});
    parser.addOption(QCommandLineOption("self-test-account", "Capture account settings during the offline self-test"));
    parser.process(application);
    if (qEnvironmentVariableIsEmpty("QT_QUICK_CONTROLS_STYLE")) {
        // Plasma integration chooses org.kde.desktop automatically when installed.
        // Respect all user style overrides and avoid forcing a foreign palette.
        if (qEnvironmentVariable("XDG_CURRENT_DESKTOP").contains("KDE", Qt::CaseInsensitive)) {
            auto roots = qEnvironmentVariable("QML_IMPORT_PATH").split(QDir::listSeparator());
            roots << QLibraryInfo::path(QLibraryInfo::QmlImportsPath);
            for (const auto &root : roots) if (QFile::exists(QDir(root).filePath("org/kde/desktop/qmldir"))) { QQuickStyle::setStyle("org.kde.desktop"); break; }
        }
    }
    auto core = parser.value("core");
    if (core.isEmpty()) core = QDir(application.applicationDirPath()).filePath("noridrive-core");
    Backend backend(core, parser.value("config"), parser.isSet("no-tray"), parser.isSet("background"));
    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty("App", &backend);
    engine.rootContext()->setContextProperty("startSettings", parser.isSet("settings"));
    engine.load(QUrl("qrc:/Main.qml"));
    if (engine.rootObjects().isEmpty()) return 1;
    application.setWindowIcon(QIcon(":/drive.svg"));
    auto *window = qobject_cast<QQuickWindow *>(engine.rootObjects().first());
    backend.powerMonitor()->watchWindow(window);
    if (parser.isSet("self-test")) {
        const auto directory = parser.value("self-test"); QDir().mkpath(directory);
        QTimer::singleShot(1500, &application, [&, directory] { window->grabWindow().save(QDir(directory).filePath("qt-status.png")); backend.injectPower(true, false); });
        QTimer::singleShot(2200, &application, [&] {
            if (parser.isSet("self-test-account")) QMetaObject::invokeMethod(window, "showAccountForTest");
            else emit backend.showSettings();
        });
        QTimer::singleShot(3200, &application, [&, directory] {
            bool ready = backend.ready() && backend.status() == backend.translate("status.power_paused");
            if (parser.isSet("self-test-account")) {
                auto *selector = window->findChild<QObject *>("authModeSelector");
                const int expected = backend.config().value("auth_mode").toString() == "browser" ? 1 : 0;
                ready = ready && selector && selector->property("currentIndex").toInt() == expected;
            }
            for (auto *candidate : application.allWindows()) if (candidate != window && candidate->isVisible()) if (auto *quick = qobject_cast<QQuickWindow *>(candidate)) quick->grabWindow().save(QDir(directory).filePath("qt-settings.png"));
            QFile result(QDir(directory).filePath("qt-smoke.json")); result.open(QIODevice::WriteOnly); result.write(QJsonDocument(QJsonObject{{"ready",ready},{"style",backend.style()},{"qt",qVersion()}}).toJson());
            backend.quit(); if (!ready) application.exit(2);
        });
    }
    return application.exec();
}
