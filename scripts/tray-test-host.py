"""Isolated StatusNotifier fixture/controller for desktop smoke tests; never accesses an account."""
import json
import sys
from pathlib import Path

import dbus
import dbus.service
from dbus.mainloop.glib import DBusGMainLoop
from gi.repository import GLib

DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
endpoint = Path("/tmp/noridrive-tray-endpoint.json")


if sys.argv[1] == "host":
    class Host(dbus.service.Object):
        @dbus.service.method("org.freedesktop.DBus.Introspectable", in_signature="", out_signature="s", path_keyword="object_path", connection_keyword="connection")
        def Introspect(self, object_path, connection):
            # Qt's dynamic QDBusInterface discovers properties through introspection.
            xml = super().Introspect(object_path, connection)
            interface = '<interface name="org.kde.StatusNotifierWatcher">'
            return xml.replace(interface, interface + '''
              <property name="IsStatusNotifierHostRegistered" type="b" access="read"/>
              <property name="ProtocolVersion" type="i" access="read"/>
              <property name="RegisteredStatusNotifierItems" type="as" access="read"/>
            ''')

        @dbus.service.method("org.kde.StatusNotifierWatcher", in_signature="s", out_signature="", sender_keyword="sender")
        def RegisterStatusNotifierItem(self, service, sender=None):
            endpoint.write_text(json.dumps({"service": str(sender if service.startswith("/") else service), "path": str(service if service.startswith("/") else "/StatusNotifierItem")}))

        @dbus.service.method("org.kde.StatusNotifierWatcher", in_signature="s", out_signature="")
        def RegisterStatusNotifierHost(self, service):
            pass

        @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="ss", out_signature="v")
        def Get(self, interface, name):
            return self.GetAll(interface)[name]

        @dbus.service.method("org.freedesktop.DBus.Properties", in_signature="s", out_signature="a{sv}")
        def GetAll(self, interface):
            return {"IsStatusNotifierHostRegistered": True, "ProtocolVersion": dbus.Int32(0), "RegisteredStatusNotifierItems": dbus.Array([], signature="s")}

    name = dbus.service.BusName("org.kde.StatusNotifierWatcher", bus)
    host = Host(name, "/StatusNotifierWatcher")
    GLib.MainLoop().run()
else:
    target = json.loads(endpoint.read_text())
    item = bus.get_object(target["service"], target["path"])
    if sys.argv[1] == "activate":
        dbus.Interface(item, "org.kde.StatusNotifierItem").Activate(dbus.Int32(1100), dbus.Int32(800))
    else:
        path = dbus.Interface(item, "org.freedesktop.DBus.Properties").Get("org.kde.StatusNotifierItem", "Menu")
        menu = dbus.Interface(bus.get_object(target["service"], path), "com.canonical.dbusmenu")
        _, layout = menu.GetLayout(0, -1, dbus.Array([], signature="s"))

        def find(node):
            item_id, properties, children = node
            if str(properties.get("label", "")) == sys.argv[1]:
                return item_id
            for child in children:
                found = find(child)
                if found is not None:
                    return found
            return None

        item_id = find(layout)
        if item_id is None:
            raise RuntimeError(f"Menu item not found: {sys.argv[1]}")
        menu.Event(item_id, "clicked", dbus.Int32(0), dbus.UInt32(0))
