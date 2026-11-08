# Trash and synchronization

With **Sync deletions in both directions** enabled, deleting an already synchronized local file moves its existing server FileNode into the account's trash. Moving a server file into the trash removes the local working copy at the next successful reconciliation. Restoring it on the server downloads it to the selected local tree again. Initial absence never counts as deletion, and a local unsynchronized edit wins over a remote deletion. Mass-deletion limits remain in effect.

New profiles enable deletion propagation. An existing profile's explicit setting is retained. Isolated debug profiles disable it. Removing a folder selection with **Stop sync** never deletes its local or remote files.

The client uses an existing directory with the `trash` role, or the root `/.Trash` used by Bulwark. On Stalwart 0.16.27 the server ignores directory roles, so the conventional root folder is necessary. When creating a trash directory on a server with the modern FileNode schema, the client sets `role: "trash"` as well. An ambiguous or invalid trash blocks the operation; permanent `FileNode/destroy` is never used.

Items go into distinct batch directories named `timestamp~originalParentId`, retaining their original filename. This interoperates with Bulwark's restore logic and avoids collisions between equal filenames from different folders. Deleted directory trees move together, preserving their hierarchy. Trash contents are excluded from normal synchronization, including on older servers that return `role: null`.

The **Recycle bin** button opens the advertised JMAP `webTrashUrl`. If the server does not advertise one, it opens the configured web client, where the trash can be selected. No client-specific web route is guessed.

Internal overwrite protection is separate: previous local versions remain under `.filenode/history`, and previous remote versions under `.filenode-history` within the selected remote folder. These stores are not the Recycle bin. They have no automatic retention policy and consume storage. Local working copies removed due to server deletion are preserved internally before removal. No history restore UI is provided yet.

Protocol references: [FileNode directory roles](https://www.ietf.org/ietf-ftp/internet-drafts/draft-ietf-jmap-filenode-20.html#section-3.1.4), [Bulwark trash layout and restore convention](https://github.com/bulwarkmail/webmail/blob/main/lib/files-trash.ts), [Stalwart 0.16.27 role handling](https://github.com/stalwartlabs/stalwart/blob/v0.16.27/crates/jmap/src/file/get.rs).
