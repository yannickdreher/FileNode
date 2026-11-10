#include <KAbstractFileItemActionPlugin>
#include <KOverlayIconPlugin>
#include <KPluginFactory>
#include <QApplication>
#include <QPluginLoader>
#include <QTimer>
#include <QDebug>
#include <memory>

int main(int argc, char **argv) {
    QApplication application(argc, argv);
    if (argc != 3) return 2;
    QPluginLoader overlayLoader(QString::fromLocal8Bit(argv[1]));
    auto *overlay = qobject_cast<KOverlayIconPlugin *>(overlayLoader.instance());
    if (!overlay) { qCritical() << overlayLoader.errorString(); return 1; }
    QPluginLoader actionsLoader(QString::fromLocal8Bit(argv[2]));
    auto *factory = qobject_cast<KPluginFactory *>(actionsLoader.instance());
    if (!factory) {qCritical() << actionsLoader.errorString(); return 1; }
    std::unique_ptr<KAbstractFileItemActionPlugin> actions(factory->create<KAbstractFileItemActionPlugin>(nullptr));
    if (!actions) return 1;
    // Exercise the asynchronous metadata callback without any account or data.
    bool completed = false;
    QObject::connect(overlay, &KOverlayIconPlugin::overlaysChanged, &application, [&](const QUrl &, const QStringList &icons) {
        completed = icons.isEmpty(); application.quit();
    });
    overlay->getOverlays(QUrl::fromLocalFile("/tmp/filenode-no-such-fixture"));
    QTimer::singleShot(3000, &application, &QCoreApplication::quit);
    application.exec();
    return completed ? 0 : 1;
}
