#include <KOverlayIconPlugin>
#include <QCoreApplication>
#include <QCache>
#include <QDateTime>
#include <QFutureWatcher>
#include <QSet>
#include <QThreadPool>
#include <QTimer>
#include <QtConcurrent>
#include <sys/xattr.h>

class FileNodeOverlay : public KOverlayIconPlugin {
    Q_OBJECT
    Q_PLUGIN_METADATA(IID "org.kde.overlayicon.filenode")
    QThreadPool pool;
    struct Cached { QStringList icons; qint64 updated; };
    QCache<QString, Cached> cache{2000};
    QSet<QString> pending;
    QTimer timer;
public:
    FileNodeOverlay() {
        // Qt can move a plugin instance between loader/application threads.
        // QObject members must follow its affinity together with the root.
        timer.setParent(this);
        pool.setParent(this);
        pool.setMaxThreadCount(2);
        timer.setInterval(3000);
        connect(&timer, &QTimer::timeout, this, [this] { for (const auto &path : cache.keys()) getOverlays(QUrl::fromLocalFile(path)); });
        timer.start();
        // Qt's loader can retain the plugin until static teardown, after the
        // application thread object is gone. Stop live objects beforehand.
        connect(QCoreApplication::instance(), &QCoreApplication::aboutToQuit, this, [this] {
            timer.stop(); pool.waitForDone();
        });
    }
    QStringList getOverlays(const QUrl &url) override {
        if (!url.isLocalFile()) return {};
        const auto path = url.toLocalFile(); const auto *previous = cache.object(path);
        const auto result = previous ? previous->icons : QStringList{};
        if ((!previous || previous->updated + 2000 < QDateTime::currentMSecsSinceEpoch()) && !pending.contains(path) && pending.size() < 256) {
            pending.insert(path);
            auto *watcher = new QFutureWatcher<QStringList>(this);
            connect(watcher, &QFutureWatcher<QStringList>::finished, this, [this, watcher, path, url] {
                pending.remove(path); const auto icons = watcher->result(); const auto *old = cache.object(path);
                const auto changed = !old || old->icons != icons;
                cache.insert(path, new Cached{icons, QDateTime::currentMSecsSinceEpoch()});
                if (changed) emit overlaysChanged(url, icons);
                watcher->deleteLater();
            });
            watcher->setFuture(QtConcurrent::run(&pool, [path] {
                char buffer[32]{}; const auto count = lgetxattr(path.toUtf8().constData(), "user.filenode.status", buffer, sizeof(buffer));
                const auto state = count > 0 ? QByteArray(buffer, count) : QByteArray{};
                if (state == "online") return QStringList{"folder-remote-symbolic"};
                if (state == "local") return QStringList{"emblem-default"};
                if (state == "pinned") return QStringList{"emblem-favorite"};
                if (state == "syncing") return QStringList{"emblem-synchronizing"};
                return QStringList{};
            }));
        }
        return result;
    }
};
#include "dolphin-overlay.moc"
