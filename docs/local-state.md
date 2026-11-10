# Local synchronization state

FileNode keeps each folder's SQLite baseline, operation journal, download/upload staging and local overwrite history in the user's local application data directory. Full synchronization and Windows Files On-Demand share the same store. Changing between these modes does not move or reset the baseline.

| System | Folder stores |
| --- | --- |
| Windows | `%LOCALAPPDATA%\FileNode\data\roots\<folder-key>` |
| Linux | `$XDG_DATA_HOME/filenode/roots/<folder-key>`, normally `~/.local/share/filenode/roots/<folder-key>` |

The key is derived from the canonical local folder path. A stored account/server/remote-path identity prevents accidentally reusing the baseline with a different account. Each store has an exclusive engine lock. Unavailable sync roots are deferred; FileNode does not recreate them or interpret a disconnected drive as deletions.

## Migration from earlier releases

The first synchronization with this version automatically migrates a selected folder's old `.filenode` directory and, if present, its previous external `cloud/<root-key>` store. Stopped folders are migrated when selected again. No new permanent `.filenode` directory is created inside a sync root.

1. Lock the new store and all existing legacy stores. A still-running older engine blocks migration until it exits.
2. Make a consistent SQLite copy, including committed WAL transactions, and verify its integrity. Copy and verify history, partial transfers and any other legacy files before removing originals.
3. Use the active cloud baseline when both old layouts exist. After an older version has detached Cloud Files, use its newer full-sync baseline instead. Preserve recovery copies from both stores.
4. Publish the external database last, then remove verified legacy files and their old directories. A retry after interrupted cleanup keeps the new external database authoritative.

Recovery copies remain under the new store's `history/migration/<source-key>/`: `database.sqlite` contains the original database, and `contents/` contains the original history, partial transfers and other files. They are not uploaded. They have no automatic retention policy and consume local disk space. Migration temporarily needs enough space for a verified copy. An identity mismatch, locked file, changed backup or insufficient disk space stops the operation instead of initializing an empty baseline; normal synchronization retries after the cause is resolved.

Do not delete either legacy or external state to repair synchronization. This can remove the evidence needed to distinguish a deletion from an initial absence. Do not run an older FileNode version against a migrated folder; it does not understand the new storage layout.

## Other drives and recovery

Local history stays in the application data directory even when the selected folder is on another drive. Cross-volume preservation copies and verifies contents before removing originals. If the copy fails, the originals are retained; if removal is interrupted, the completed recovery copy remains available.

Completed downloads are published atomically on the destination volume. When the volumes differ, this requires a short-lived sibling file named `.filenode-transfer-*.part` in the destination folder. These names are always excluded from synchronization, independently of user-editable exclusions. A process crash can leave such a temporary file; it is not a state directory.

Removing a selection retains its external state and history for later re-adding. [Server trash](deletions.md) is separate from these internal local recovery copies. Independent backups remain necessary.
