# File and folder exclusions

In **Synchronization → Excluded files and folders**, edit one pattern per line and select **Save settings**. You can edit while syncing; FileNode safely applies the saved rules and continues automatically. Rules apply to every selected folder and in both directions. An empty list disables optional exclusions. Invalid patterns are rejected before saving.

Defaults: `~$*`, `*.tmp`, `*.temp`, `*.swp`, `*.swo`, `*~`, `*.crdownload`, `*.part`, `*.partial`, `*.download`, `desktop.ini`, `Thumbs.db`, `.DS_Store`. These cover typical Office owner/lock files, editor scratch files, partial downloads and desktop metadata. A legitimate file can also have one of these names; remove the relevant rule if you want to sync it. Ordinary documents, archives and executable files are not generally excluded by extension. Active databases and VM disks need an application-consistent backup or snapshot; a settled timestamp alone does not establish consistency.

Patterns are case-insensitive and Unicode-normalized. Without `/`, a pattern matches a name at any depth, including directories and everything below them. With `/`, it is relative to each selected root. `*`, `?`, character classes and `**` use glob syntax; negation and absolute paths are not supported. Use forward slashes on both operating systems. Limits: 128 patterns, 256 UTF-8 bytes per pattern.

| Pattern | Effect |
| --- | --- |
| `*.tmp` | Omit matching names anywhere |
| `Cache` | Omit names called Cache and their contents |
| `Reports/**/*.part` | Omit partial files under Reports |

Exclusions are not deletions. Previously synchronized excluded entries keep their stored baselines and their existing local/server copies. Excluded local trees are skipped before file hashing; matching remote entries are filtered before planning. Directory deletes, conflicts and renames involving excluded descendants are blocked, while ordinary sibling edits can continue. Removing a rule re-enables the normal three-way reconciliation, including detection of changes or deletions made while excluded; it does not reset history or force an upload.

Private state/history, trash and `.git` have separate mandatory protections and remain excluded even with an empty optional list. This is a synchronization policy, not an access-control or data-loss-prevention boundary.
