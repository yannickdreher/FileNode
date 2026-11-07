#pragma once
#include <QObject>
#include <QVariantMap>
#ifdef Q_OS_WIN
#include <QAbstractNativeEventFilter>
#endif

class QWindow;
class PowerMonitor : public QObject
#ifdef Q_OS_WIN
    , public QAbstractNativeEventFilter
#endif
{
    Q_OBJECT
public:
    explicit PowerMonitor(QObject *parent = nullptr);
    void watchWindow(QWindow *window);
    bool saver = false;
    bool sleeping = false;
    bool available = false;
signals:
    void changed();
private slots:
    void refresh();
    void prepareForSleep(bool value);
    void propertiesChanged(const QString &, const QVariantMap &, const QStringList &);
#ifdef Q_OS_WIN
private:
    bool nativeEventFilter(const QByteArray &, void *, qintptr *) override;
#endif
};
