#pragma once
#include "power.h"
#include <QObject>
#include <QProcess>
#include <QSystemTrayIcon>
#include <QVariantMap>
#include <QVariantList>
#include <QMenu>
#include <QJsonObject>
#include <QAbstractListModel>
#include <QDateTime>
#include <QFileInfo>

class ActivityModel : public QAbstractListModel {
    Q_OBJECT
    Q_PROPERTY(int count READ count NOTIFY countChanged)
public:
    using QAbstractListModel::QAbstractListModel;
    int count() const { return rows.size(); }
    int rowCount(const QModelIndex &parent = {}) const override { return parent.isValid() ? 0 : rows.size(); }
    QVariant data(const QModelIndex &index, int role) const override { return index.isValid() && index.row() < rows.size() && role == Qt::UserRole ? rows[index.row()] : QVariant{}; }
    QHash<int, QByteArray> roleNames() const override { return {{Qt::UserRole, "activity"}}; }
    void clear() { beginResetModel(); rows.clear(); endResetModel(); emit countChanged(); }
    void requireSignIn() {
        for (int index = 0; index < rows.size(); index++) {
            auto row = rows[index].toMap(); const auto state = row.value("state").toString();
            if (state == "transferring" || state == "retrying") { row["state"] = "auth_required"; rows[index] = row; emit dataChanged(this->index(index), this->index(index)); }
        }
    }
    void renamed(const QString &folder, const QString &from, const QString &to) {
        QList<QVariantMap> moved;
        for (int index = rows.size() - 1; index >= 0; index--) {
            auto row = rows[index].toMap(); const auto path = row.value("path").toString();
            if (row.value("folder") == folder && (path == from || path.startsWith(from + "/"))) {
                row["path"] = to + path.mid(from.size()); row["state"] = "checking"; row["operation"] = "rename";
                beginRemoveRows({}, index, index); rows.removeAt(index); endRemoveRows(); moved.append(row);
            }
        }
        for (const auto &row : moved) upsert(row);
    }
    void verifiedFolder(const QString &folder) {
        for (int index = 0; index < rows.size(); index++) {
            auto row = rows[index].toMap(); const auto state = row.value("state").toString();
            if (row.value("folder") == folder && (state == "waiting" || state == "checking" || state == "retrying" || state == "paused" || state == "auth_required")) { row["state"] = "synced"; row["completed"] = row.value("total"); rows[index] = row; emit dataChanged(this->index(index), this->index(index)); }
        }
    }
    void upsert(QVariantMap value) {
        for (int index = 0; index < rows.size(); index++) {
            const auto old = rows[index].toMap();
            if (old.value("folder") == value.value("folder") && old.value("path") == value.value("path")) {
                value["time"] = old.value("state") == value.value("state") ? old.value("time") : QDateTime::currentDateTime().toString(Qt::ISODate);
                rows[index] = value; emit dataChanged(this->index(index), this->index(index)); return;
            }
        }
        value["time"] = QDateTime::currentDateTime().toString(Qt::ISODate);
        beginInsertRows({}, 0, 0); rows.prepend(value); endInsertRows();
        if (rows.size() > 100) { beginRemoveRows({}, 100, 100); rows.removeLast(); endRemoveRows(); }
        emit countChanged();
    }
signals: void countChanged();
private: QVariantList rows;
};

class Backend : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantMap config READ config NOTIFY configChanged)
    Q_PROPERTY(bool running READ running NOTIFY changed)
    Q_PROPERTY(bool ready READ ready NOTIFY changed)
    Q_PROPERTY(QString status READ status NOTIFY changed)
    Q_PROPERTY(QString error READ error NOTIFY changed)
    Q_PROPERTY(ActivityModel* activities READ activities CONSTANT)
    Q_PROPERTY(QString accountState READ accountState NOTIFY changed)
    Q_PROPERTY(QString accountStatus READ accountStatus NOTIFY changed)
    Q_PROPERTY(bool authenticated READ authenticated NOTIFY changed)
    Q_PROPERTY(bool powerPaused READ powerPaused NOTIFY changed)
    Q_PROPERTY(QVariantMap folderStatuses READ folderStatuses NOTIFY changed)
    Q_PROPERTY(QString language READ language NOTIFY configChanged)
    Q_PROPERTY(bool trayAvailable READ trayAvailable NOTIFY changed)
    Q_PROPERTY(bool powerAvailable READ powerAvailable NOTIFY changed)
    Q_PROPERTY(QString style READ style CONSTANT)
    Q_PROPERTY(QString version READ version CONSTANT)
public:
    explicit Backend(QString core, QString profile, bool noTray, bool background, QObject *parent = nullptr);
    ~Backend() override;
    QVariantMap config() const { return m_config; }
    bool running() const { return m_running; }
    bool ready() const { return m_ready; }
    QString status() const;
    QString error() const { return m_error; }
    ActivityModel *activities() { return &m_activity; }
    QString accountState() const { return m_account; }
    QString accountStatus() const { return translate("account." + m_account); }
    bool authenticated() const { return m_account == "signed_in"; }
    bool powerPaused() const { return m_powerPaused; }
    QVariantMap folderStatuses() const { return m_folders; }
    Q_INVOKABLE QString folderStatus(const QString &folder) const;
    QString language() const;
    bool trayAvailable() const { return m_trayAvailable; }
    bool powerAvailable() const { return m_power.available; }
    QString style() const;
    QString version() const;
    Q_INVOKABLE QString translate(const QString &key) const;
    Q_INVOKABLE QString formatMessage(const QVariantMap &message) const;
    Q_INVOKABLE void apply(const QVariantMap &values, bool connectAccount = false, QString password = {}, bool interactive = false);
    Q_INVOKABLE void start();
    Q_INVOKABLE void pause();
    Q_INVOKABLE void check();
    Q_INVOKABLE void openFolder();
    Q_INVOKABLE void openPath(const QString &path);
    Q_INVOKABLE void openOnline();
    Q_INVOKABLE void openTrash();
    Q_INVOKABLE void quit();
    Q_INVOKABLE QPoint popupPosition(int width, int height) const;
    Q_INVOKABLE void injectPower(bool saver, bool sleeping); // Also used by isolated smoke tests.
    Q_INVOKABLE QString localPath(const QUrl &url) const;
    PowerMonitor *powerMonitor() { return &m_power; }
signals:
    void changed();
    void configChanged();
    void showStatus();
    void showSettings();
    void showFolders();
    void hideStatus();
private:
    void send(QJsonObject command);
    void receive();
    void updateTray();
    void publishPower();
    QProcess m_core;
    QByteArray m_buffer;
    QVariantMap m_config;
    ActivityModel m_activity;
    QVariantMap m_folders;
    QString m_account = "signed_out", m_trashUrl;
    bool m_powerPaused = false;
    QJsonObject m_en, m_de;
    QSystemTrayIcon m_tray;
    QMenu m_menu;
    QAction *m_show, *m_settings, *m_toggle, *m_check, *m_open, *m_quit;
    PowerMonitor m_power;
    bool m_running = false, m_ready = false, m_trayAvailable = false, m_background = false, m_quitting = false;
    QString m_status = "status.ready", m_error;
    bool m_connectAfterSave = false;
    bool m_pendingInteractive = false;
    QString m_pendingPassword;
};
