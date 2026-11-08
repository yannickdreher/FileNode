# Selecting folders

One profile connects one account and up to 64 local folders. In **Sync and backup**, select **Add folder**. The selection is saved immediately and the generated server path appears below it. The list remains editable while syncing. Changes safely stop the previous worker before a replacement can access the same folder databases. Sync continues automatically; adding a folder also starts it when the account is signed in. Otherwise, **Sign in** starts it. The tray menu also provides **Start syncing**.

New selections use their final folder name at the server root:

| Local selection | Server path |
| --- | --- |
| `C:\Users\Yannick\Desktop` | `/Desktop` |
| `C:\Users\Yannick\Documents\Büro` | `/Büro` |
| `/home/user/Documents/Office` | `/Office` |
| `D:\Projects` (outside the home directory) | `/Projects` |

The selected server folder is resolved or created at the root. Only the selected leaf's tree is synchronized: selecting `Documents/Büro` synchronizes it to `/Büro` without including its local parent or siblings. Server paths are displayed, not editable. Filesystem roots, the entire home directory, reserved names, overlapping local trees and overlapping server paths are rejected. Names must be portable to Windows and Linux. Two selections with the same name cannot share a profile. Previously saved server paths keep their existing binding; this change does not move server data or reset baselines.

**Stop sync** removes the selection without deleting local files, remote files or recovery history. No remote delete operation is performed by removing a selection. A stopped local folder keeps its `.filenode` state. Re-adding it to the same account and path resumes its existing three-way baseline; moving it to a different account or server path is refused by the store identity checks.

Each selection has its own watcher, SQLite state, settling observations and retry deadline. An unavailable local drive is deferred without recreating its root; other selections continue. Failures retry with backoff from 5 seconds to 5 minutes. **Check now** retries all selections immediately. A selected server folder that was previously pinned by ID must not silently be recreated if it disappears.

Credentials, the server connection and upload/download budgets are shared by the account. Blob uploads can run concurrently inside a bounded batch; folders, downloads and metadata commits remain serial. Adding folders or upload connections does not multiply the rate limit. See [transfers](transfers.md) and [exclusions](exclusions.md). Power saver, sleep and Pause apply to all selections.

## Headless configuration

Put account-wide options before the array-of-table entries in TOML. Leave `remote_path` absent to derive it when loading the profile. The desktop stores the generated paths to keep bindings stable.

```toml
server = "https://jmap.example.com"
web_url = "https://mail.example.com"
username = "user@example.org"
propagate_deletes = false

[[sync_folders]]
local_folder = '/home/user/Desktop'

[[sync_folders]]
local_folder = '/home/user/Documents/Büro'
```

`--dry-run` outputs an array containing each selection's local folder, remote path and planned actions. It does not create remote folders. `--once` reports failure if any selected folder fails, even when other folders succeeded.

FileNode uses a fresh application namespace. Profiles and credentials from earlier app names are not imported automatically.
