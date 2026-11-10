"""Nautilus 4 status column, emblems and Files On-Demand actions."""
from concurrent.futures import ThreadPoolExecutor
import json
import locale
import os
from pathlib import Path
import socket
import weakref

import gi
gi.require_version("Nautilus", "4.0")
from gi.repository import GObject, GLib, Nautilus

GERMAN = (locale.getlocale()[0] or "en").startswith("de")
LABELS = {
    "online": ("Available online", "Online verfügbar"),
    "local": ("Available on this device", "Auf diesem Gerät verfügbar"),
    "pinned": ("Always available offline", "Immer offline verfügbar"),
    "syncing": ("Waiting for synchronization", "Wartet auf Synchronisation"),
    "pin": ("Always keep on this device", "Immer auf diesem Gerät behalten"),
    "evict": ("Free up space", "Speicherplatz freigeben"),
    "column": ("FileNode availability", "FileNode-Verfügbarkeit"),
}
EMBLEMS = {"online": "folder-remote-symbolic", "local": "emblem-default", "pinned": "emblem-favorite", "syncing": "emblem-synchronizing"}

def label(key):
    return LABELS[key][1 if GERMAN else 0]

def status(path):
    try: return os.getxattr(path, "user.filenode.status", follow_symlinks=False).decode("ascii")
    except (OSError, UnicodeError): return ""

def command(action, path):
    directory = Path(os.environ.get("XDG_RUNTIME_DIR", "/nonexistent")) / "filenode"
    for endpoint in directory.glob("*.sock"):
        try:
            with socket.socket(socket.AF_UNIX) as connection:
                connection.settimeout(1.0); connection.connect(str(endpoint))
                connection.sendall((json.dumps({"action": action, "path": path}) + "\n").encode())
                response = connection.makefile("rb").readline(32768)
                if json.loads(response).get("ok"): return True
        except (OSError, ValueError): continue
    return False

class FileNode(GObject.GObject, Nautilus.InfoProvider, Nautilus.ColumnProvider, Nautilus.MenuProvider):
    __gtype_name__ = "FileNodeNautilusExtension"
    def __init__(self):
        super().__init__()
        self.pool = ThreadPoolExecutor(max_workers=2, thread_name_prefix="filenode-info")
        self.pending, self.visible, self.known = {}, {}, {}
        GLib.timeout_add_seconds(3, self.refresh_visible)

    def get_columns(self):
        return [Nautilus.Column(name="FileNode::availability", attribute="filenode::availability", label=label("column"), description=label("column"))]

    def update_file_info_full(self, provider, handle, closure, file):
        path = file.get_location().get_path()
        if not path or file.get_uri_scheme() != "file" or len(self.pending) >= 256:
            return Nautilus.OperationResult.COMPLETE
        future = self.pool.submit(status, path)
        self.pending[handle] = future
        future.add_done_callback(lambda future: GLib.idle_add(self.complete, provider, handle, closure, file, path, future))
        return Nautilus.OperationResult.IN_PROGRESS

    def complete(self, provider, handle, closure, file, path, future):
        if self.pending.get(handle) is not future: return False
        del self.pending[handle]
        state = future.result()
        if state in EMBLEMS:
            file.add_emblem(EMBLEMS[state]); file.add_string_attribute("filenode::availability", label(state))
            if len(self.visible) >= 1000:
                oldest = next(iter(self.visible)); self.visible.pop(oldest); self.known.pop(oldest, None)
            self.visible[path] = weakref.ref(file); self.known[path] = state
        else:
            self.visible.pop(path, None); self.known.pop(path, None)
        Nautilus.info_provider_update_complete_invoke(closure, provider, handle, Nautilus.OperationResult.COMPLETE)
        return False

    def cancel_update(self, provider, handle):
        future = self.pending.pop(handle, None)
        if future: future.cancel()

    def refresh_visible(self):
        for path, reference in list(self.visible.items()):
            file = reference()
            if file is None: self.visible.pop(path, None); self.known.pop(path, None)
            else: file.invalidate_extension_info()
        return True

    def get_file_items(self, files):
        paths = [file.get_location().get_path() for file in files]
        # Use completed metadata only. Never block the Nautilus UI on a mount.
        if not paths or any(path not in self.known for path in paths): return []
        items = []
        for action in ("pin", "evict"):
            item = Nautilus.MenuItem(name="FileNode::" + action, label=label(action), tip=label(action))
            item.connect("activate", lambda _, action=action: self.pool.submit(self.apply_action, action, paths))
            items.append(item)
        return items

    def apply_action(self, action, paths):
        for path in paths: command(action, path)
