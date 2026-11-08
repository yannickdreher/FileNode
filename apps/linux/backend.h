#pragma once
#include "power.h"
#include <QObject>
#include <QProcess>
#include <QSystemTrayIcon>
#include <QVariantMap>
#include <QVariantList>
#include <QMenu>
#include <QJsonObject>

class Backend : public QObject {
    Q_OBJECT
    Q_PROPERTY(QVariantMap config READ config NOTIFY configChanged)
    Q_PROPERTY(bool running READ running NOTIFY changed)
    Q_PROPERTY(bool ready READ ready NOTIFY changed)
    Q_PROPERTY(QString status READ status NOTIFY changed)
    Q_PROPERTY(QString error READ error NOTIFY changed)
    Q_PROPERTY(QVariantList activities READ activities NOTIFY changed)
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
    QVariantList activities() const { return m_activity; }
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
    Q_INVOKABLE void openRecovery();
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
    QVariantList m_activity;
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
