#include <KAbstractFileItemActionPlugin>
#include <KFileItemListProperties>
#include <KPluginFactory>
#include <QAction>
#include <QDir>
#include <QFile>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLocalSocket>
#include <QLocale>
#include <QProcess>
#include <QStandardPaths>
#include <QTimer>
#include <QWidget>

class FileNodeActions : public KAbstractFileItemActionPlugin {
    Q_OBJECT
public:
    explicit FileNodeActions(QObject *parent, const QVariantList &) : KAbstractFileItemActionPlugin(parent) {}
    QList<QAction *> actions(const KFileItemListProperties &selection, QWidget *parent) override {
        const auto urls = selection.urlList(); if (urls.isEmpty()) return {};
        // Inspect the kernel's topology text only. QStorageInfo::mountedVolumes
        // can stat remote filesystems and must not run in this UI callback.
        QFile topology("/proc/self/mountinfo");
        if (!topology.open(QIODevice::ReadOnly)) return {};
        QStringList roots;
        for (const auto &line : topology.read(1024 * 1024).split('\n')) {
            const auto separator = line.indexOf(" - "); if (separator < 0) continue;
            const auto filesystem = line.mid(separator + 3).split(' ').first();
            const auto fields = line.left(separator).split(' ');
            if (filesystem != "fuse.filenode" || fields.size() < 5) continue;
            roots.append(QString::fromUtf8(fields[4]).replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n").replace("\\134", "\\"));
        }
        for (const auto &url : urls) {
            bool owned = false;
            if (url.isLocalFile()) for (const auto &root : roots) {
                if (url.toLocalFile() == root || url.toLocalFile().startsWith(root + '/')) { owned = true; break; }
            }
            if (!owned) return {};
        }
        const bool german = QLocale::system().language() == QLocale::German;
        QList<QAction *> result;
        for (const auto &command : {QString("pin"), QString("evict")}) {
            auto *action = new QAction(command == "pin" ? (german ? "Immer auf diesem Gerät behalten" : "Always keep on this device") : (german ? "Speicherplatz freigeben" : "Free up space"), parent);
            connect(action, &QAction::triggered, this, [this, urls, command] {
                for (const auto &url : urls) dispatch(command, url.toLocalFile());
            });
            result.append(action);
        }
        return result;
    }
private:
    void dispatch(const QString &action, const QString &path) {
        const auto directory = QDir(QStandardPaths::writableLocation(QStandardPaths::RuntimeLocation) + "/filenode");
        for (const auto &endpoint : directory.entryList({"*.sock"}, QDir::Files | QDir::System)) {
            auto *socket = new QLocalSocket(this);
            connect(socket, &QLocalSocket::connected, socket, [socket, action, path] { socket->write(QJsonDocument(QJsonObject{{"action",action},{"path",path}}).toJson(QJsonDocument::Compact) + '\n'); });
            connect(socket, &QLocalSocket::readyRead, socket, [this, socket] {
                if (!socket->canReadLine()) return;
                const auto response = QJsonDocument::fromJson(socket->readLine(32768)).object();
                // A different active mount rejects this path. Only its owning
                // endpoint accepts the request; no credentials are exchanged.
                Q_UNUSED(response)
                socket->disconnectFromServer(); socket->deleteLater();
            });
            connect(socket, &QLocalSocket::errorOccurred, socket, &QObject::deleteLater);
            QTimer::singleShot(2000, socket, &QObject::deleteLater);
            socket->connectToServer(directory.filePath(endpoint));
        }
    }
};
K_PLUGIN_CLASS_WITH_JSON(FileNodeActions, "dolphin-actions.json")
#include "dolphin-actions.moc"
