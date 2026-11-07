#include "power.h"
#include <QCoreApplication>
#include <QWindow>
#ifdef Q_OS_WIN
#include <windows.h>
#else
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusVariant>
#include <QDBusServiceWatcher>
#endif

PowerMonitor::PowerMonitor(QObject *parent) : QObject(parent) {
#ifdef Q_OS_WIN
    QCoreApplication::instance()->installNativeEventFilter(this);
#else
    auto bus = QDBusConnection::systemBus();
    bus.connect("org.freedesktop.login1", "/org/freedesktop/login1", "org.freedesktop.login1.Manager", "PrepareForSleep", this, SLOT(prepareForSleep(bool)));
    const QStringList services {"org.freedesktop.UPower.PowerProfiles", "net.hadess.PowerProfiles"};
    for (const auto &service : services) {
        const auto path = service == services.first() ? "/org/freedesktop/UPower/PowerProfiles" : "/net/hadess/PowerProfiles";
        bus.connect(service, path, "org.freedesktop.DBus.Properties", "PropertiesChanged", this, SLOT(propertiesChanged(QString,QVariantMap,QStringList)));
        auto *watcher = new QDBusServiceWatcher(service, bus, QDBusServiceWatcher::WatchForOwnerChange, this);
        connect(watcher, &QDBusServiceWatcher::serviceOwnerChanged, this, [this] { refresh(); });
    }
#endif
    refresh();
}
void PowerMonitor::watchWindow(QWindow *window) {
#ifdef Q_OS_WIN
    static const GUID saving {0xe00958c0, 0xc213, 0x4ace, {0xac,0x77,0xfe,0xcc,0xed,0x2e,0xee,0xa5}};
    RegisterPowerSettingNotification(reinterpret_cast<HANDLE>(window->winId()), &saving, DEVICE_NOTIFY_WINDOW_HANDLE);
#else
    Q_UNUSED(window)
#endif
}
void PowerMonitor::refresh() {
#ifdef Q_OS_WIN
    SYSTEM_POWER_STATUS status {};
    available = GetSystemPowerStatus(&status);
    // The fourth byte was named Reserved1 in older MinGW headers; Windows 10+
    // defines the same ABI byte as SystemStatusFlag (energy saver enabled).
    if (available) saver = reinterpret_cast<const unsigned char *>(&status)[3] == 1;
#else
    auto bus = QDBusConnection::systemBus();
    available = false;
    if (bus.isConnected()) for (const auto &service : {QString("org.freedesktop.UPower.PowerProfiles"), QString("net.hadess.PowerProfiles")}) {
        if (!bus.interface()->isServiceRegistered(service).value()) continue;
        auto path = service.startsWith("org.") ? "/org/freedesktop/UPower/PowerProfiles" : "/net/hadess/PowerProfiles";
        auto message = QDBusMessage::createMethodCall(service, path, "org.freedesktop.DBus.Properties", "Get");
        message << service << "ActiveProfile";
        auto reply = bus.call(message, QDBus::Block, 1000);
        if (reply.type() == QDBusMessage::ReplyMessage && !reply.arguments().isEmpty()) {
            saver = qvariant_cast<QDBusVariant>(reply.arguments().first()).variant().toString() == "power-saver";
            available = true; break;
        }
    }
#endif
    emit changed();
}
void PowerMonitor::prepareForSleep(bool value) { sleeping = value; emit changed(); }
void PowerMonitor::propertiesChanged(const QString &, const QVariantMap &changedProperties, const QStringList &invalidated) {
    if (changedProperties.contains("ActiveProfile") || invalidated.contains("ActiveProfile")) refresh();
}
#ifdef Q_OS_WIN
bool PowerMonitor::nativeEventFilter(const QByteArray &, void *data, qintptr *) {
    const auto *message = static_cast<MSG *>(data);
    if (message->message == WM_POWERBROADCAST) {
        if (message->wParam == PBT_APMSUSPEND) prepareForSleep(true);
        else if (message->wParam == PBT_APMRESUMEAUTOMATIC || message->wParam == PBT_APMRESUMESUSPEND) { prepareForSleep(false); refresh(); }
        else if (message->wParam == PBT_POWERSETTINGCHANGE || message->wParam == PBT_APMPOWERSTATUSCHANGE) refresh();
    }
    return false; // Never inhibit sleep or override OS power behavior.
}
#endif
