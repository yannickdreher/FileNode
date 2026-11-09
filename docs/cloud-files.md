# Windows Files On-Demand

Windows builds can register each selected folder as a native Cloud Files sync root. Enable **Files On-Demand (Windows Explorer)** in Sync and backup. Existing profiles default to full local synchronization; enabling this feature does not automatically evict existing files. Linux continues using complete local files.

Use an existing, dedicated **local NTFS folder**, outside OneDrive and other cloud providers. Windows prohibits overlapping sync roots. An ordinary folder such as `%USERPROFILE%\FileNode\Documents` is suitable; a redirected Documents folder inside OneDrive is not. FileNode reports the conflicting provider before saving the setting. No administrator rights, certificate import, custom filesystem driver or system service is required.

## Explorer behavior

- The registered FileNode folder appears in Explorer's navigation pane with the app icon. Windows owns the availability icons, status column and standard cloud-file context menu.
- Server files first appear as online-only placeholders. Listing a folder, scanning it, or displaying FileNode's activity rows does not fetch their contents. Activity thumbnails are requested from the existing Windows cache only.
- Opening a file downloads its requested contents through authenticated JMAP blob requests. Applications can seek to arbitrary offsets. Transfers are page aligned, bounded in memory, cancellable, and report progress to Explorer and the FileNode activity view.
- **Always keep on this device** pins files or whole folder trees and downloads missing contents. Pinned files remain available offline.
- **Free up space** requests eviction of committed contents. Windows and FileNode refuse to evict unsynchronized local edits. The placeholder, server object and three-way baseline remain present; eviction never counts as deletion.
- Storage Sense may evict eligible unpinned, synchronized files through the native Windows policy.
- FileNode must be running to open online-only files. Quit preserves the registrations and placeholders; restart reconnects them. Enable login startup in General for normal daily use. Fully downloaded files remain readable offline.

The built-in Explorer actions are the native Windows commands; this does not add a separate legacy icon-overlay extension. Sharing links, account-specific server recycle-bin commands, custom thumbnail providers and Microsoft Store publication are separate features.

## Synchronization and interruptions

The ordinary three-way planner still controls uploads, conflicts, renames and trash. A placeholder stores its committed server ID/blob version and stable local comparison identity. Hydration, eviction and metadata-only renames do not change that comparison identity. Local content changes are hashed and uploaded; only successful server commits can mark them in sync. Updating a remote online-only file invalidates its old contents without downloading the obsolete version first.

Cloud callbacks use the same authentication and shared download payload budget as normal synchronization. Valid HTTP `206 Content-Range` responses are checked precisely; servers that ignore Range are supported with a bounded-memory streaming fallback. A server without Range support can require retransmitting the preceding bytes. JMAP notifications and polling continue to refresh the namespace. The Windows filter retains completed hydration ranges across interrupted requests, so a later open can request the remaining ranges. Uploads still restart the affected whole blob; Cloud Files does not add a resumable-upload protocol or block-level upload deltas.

Network errors, cancellation and power policy interrupt requests without marking a different server version committed. Explorer reports an unavailable download, and a later open or pin attempt retries. Native diagnostics use event `cloud_hydration_retry`. A configured additional file-size limit applies to on-demand downloads too.

One account session shares its HTTP connection pool across folders, uploads and hydration requests. Folder errors retain that session and respect their own retry deadlines; they do not restart discovery or the push connection in a tight loop. Transfer handles do not retain a replaced push worker. A refreshed account session is applied to subsequent cloud requests, while each already running request keeps its immutable blob version.

## State, stopping sync and uninstalling

Cloud mode stores databases, transfer staging and overwrite history outside the managed root, under the per-user local FileNode data directory in `cloud/<root-key>`. If the root is on a different volume, recovery moves use its excluded `.filenode/history` directory to remain atomic; completed downloads are copied to excluded staging on that volume before installation. Enabling an existing full-sync root copies its consistent SQLite baseline under both folder locks. The previous `.filenode` directory remains intact and excluded. Do not delete either state directory to repair sync.

Before **Stop sync** or disabling Files On-Demand, choose **Always keep on this device** for the selected root and wait for completion. FileNode refuses to detach online-only files. With every byte local, it reverts its placeholders, unregisters that root and transfers the latest baselines back into ordinary local state. Unuploaded local edits remain dirty and are uploaded if full synchronization continues. Both local and server files are retained. The EXE uninstaller refuses removal while FileNode cloud roots are registered; disable the feature in the app first.

## Validation

Windows CI runs real Cloud Files operations in isolated NTFS fixtures with an in-memory JMAP model. It covers metadata-only population, hydration/eviction, dirty-file protection, uploaded-file conversion, remote replacement, renames without content downloads, local/server trash, recursive pinning, database migration, re-enabling, provider restart and lossless detach. Loopback HTTP tests additionally cover exact ranges, ignored Range, invalid ranges and cancellation before sending.

Run native tests explicitly (the Cloud Files driver must already be running locally):

```powershell
cargo test --locked --lib cloud::windows::tests:: -- --ignored --test-threads=1
```

Live multi-device acceptance with a real JMAP account, long offline periods, disk exhaustion and Explorer accessibility remains part of preview testing. The existing namespace/name limits and lack of ACL, alternate-stream and timestamp replication still apply. This is not a claim of complete OneDrive product parity.

References: [Microsoft Cloud Files overview](https://learn.microsoft.com/en-us/windows/win32/cfapi/build-a-cloud-file-sync-engine), [placeholder updates and race protection](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/nf-cfapi-cfupdateplaceholder), [hydration policies](https://learn.microsoft.com/en-us/windows/win32/api/cfapi/ns-cfapi-cf_sync_policies).
