# Architecture and acceptance criteria

## Boundaries

`config` contains a password-free profile. `i18n` owns presentation language and structured messages. `scanner` invalidates an in-memory fingerprint/hash cache from native events, debounces bursts and falls back to complete content verification when events are unavailable or lost. `filesystem` validates portable names, detects unsafe links and hashes full contents. `jmap` is the only HTTP adapter. `planner` is a pure three-way comparison. `engine` applies a validated plan and rechecks state at mutation boundaries. `store` owns SQLite baseline/journal metadata and the exclusive local-folder lock. The egui desktop uses a worker thread, a compact status viewport, a separate settings viewport and a native notification-area icon. Tray callbacks wake egui rather than forcing a permanent repaint loop. Closing the status window hides it when a usable tray host exists; explicit quit terminates the worker. Linux host availability is checked over D-Bus. Optional autostart writes only the current user Run key (Windows) or XDG autostart file (Linux), only when the checkbox is changed. The desktop worker keeps its JMAP connection across successful cycles; credentials stay out of configuration and are loaded through the OS keyring. Headless mode uses the same engine.

## Synchronization invariants

1. A complete local namespace scan and consistent remote snapshot precede planning. Missing/failed listings are errors, never empty listings.
2. Initial absence cannot imply deletion. Deletion needs a previous baseline and explicit enablement. Changed content wins over deletion. A dirty descendant prevents deletion of its directory.
3. Remote overwrites, moves and creates include the snapshot-derived `ifInState`. A recovery copy is created before replacing remote content. No retry blindly repeats a mutating request whose outcome is uncertain.
4. Final local verification checks content again before replacement. Downloads are staged and fsynced, then renamed on the same filesystem. Previous local contents are moved to history; conflicts remain inside the sync tree as siblings. This reduces races but is not a transaction against other local applications.
5. A remote root is pinned by ID after the initial binding. A missing root cannot cause recreation or mass local deletion.
6. A profile identity binds server, username, explicit account selection, canonical local path and remote folder. Discovery additionally pins the actual account ID. Only one process owns a local state store.
7. State changes follow completed file operations. On restart, current local/remote state is inspected again; a pending journal entry is evidence, not an instruction to blindly replay. Some recovery copies can be duplicated after a crash; preserving content takes priority over deduplicating history.
8. No authenticated request follows a different origin (scheme, host or port). Discovery redirects are bounded and manually checked. Advertised endpoint templates are checked before use. Passwords are never written to logs/config; server error payloads are not dumped.
9. Remote paths cannot escape the chosen folder, use Windows special names, contain path separators, collide after NFC/case folding or traverse symlinks/reparse points. The account and local user are trusted: the client is not a sandbox against a malicious process with the same filesystem permissions. OS-level permission/ACL isolation and independent backups remain necessary.
10. Language changes affect rendering only. English keys, persisted paths, protocol schema, conflict suffixes and three-way decisions do not vary by locale. Arguments are substituted once so braces in file names cannot rewrite other arguments.

## Test coverage

Unit/property tests exercise the three-way matrix, mass-deletion limits, first-sync behavior, simultaneous edits, path traversal, reserved Windows names, normalization collisions, content hashing, symlink rejection, catalog parity, locale selection and live message rendering. Scanner tests exercise zero rehash on idle, same-size/same-mtime edits signaled by events, overflow/error invalidation and ignored internal paths. An integration test asserts that idle cycles do not rewrite unchanged baseline/snapshot records. Loopback HTTP contract tests cover session discovery, legacy/modern schemas, paged snapshots, changes, uploads, conditional writes, resumed downloads and endpoint origin containment.

Stateful integration tests use real files/SQLite plus immutable in-memory remote blobs. They cover idempotence, initial upload, remote edits, both-side conflicts, ambiguous create after server commit, overwrite backups, both deletion directions, delete-versus-edit, local and remote renames, failed listings, concurrent remote state changes, missing roots, profile/lock protection and file/directory conflicts.

The isolated desktop smoke test registers a real KSNI tray over D-Bus, closes the window without exiting, reactivates it through the tray, opens the settings viewport and quits through the menu. It does not exercise the Windows Explorer shell.

These tests do not establish live server interoperability or arbitrary power-loss durability. SQLite synchronous FULL and file fsync are used, but all containing directory entries are not explicitly fsynced. Remote blob IDs are opaque; downloads validate TLS, advertised length and local SHA-256, without an independent server-provided content digest. Existing full-size staged downloads assume the local private directory is trustworthy.

## Follow-up acceptance work

- Exercise an isolated Stalwart 0.16.27 account and the intended newer server version through Bulwark and two desktop devices. Verify quotas, FileNode limits, hidden recovery-folder visibility, app-password permissions and changes retention.
- Run interrupted-transfer, disk-full, locked-file, database corruption, network partition, sleep/resume and forced-kill campaigns on native Windows NTFS and Linux ext4. Record outcomes before broad deployment.
- Expand directory/case-only rename handling, robust local race isolation, account/snapshot consistency validation and history restore/indexing. Add Retry-After support and jitter, concurrent transfer budgeting and server push invalidation while retaining periodic full reconciliation. Local namespace enumeration remains O(n); the cache reduces content I/O, not directory-walk complexity. The cache is intentionally not trusted across process restarts, and all files are rehashed at startup and after reconnecting.
- Define retention, storage quotas, ignored patterns, selective sync and a recoverable conflict/restore workflow.
- Implement OAuth with PKCE and system browser if the installed Stalwart supports the required flows; never embed a client secret in the binary.
- Implement files on demand separately per OS (Windows Cloud Files API; a Linux virtual filesystem), including hydration/eviction, dirty-file pinning, offline behavior and permission models.
- Add native Windows CI execution, additional Linux desktop integration, signed installers, update verification, dependency/security scanning and release acceptance gates.

Until these gates pass, distribute this client as a preview and validate a dedicated test folder before connecting valuable data.
