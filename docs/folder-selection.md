# Selecting folders

One profile connects one account and up to 64 local folders. In **Sync and backup**, select **Add folder**. The selection is saved immediately and the generated server path appears below it. Pause synchronization before changing the list; use **Sign in** or **Resume** to start it afterwards.

Paths inside the user's home directory retain their relative structure:

| Local selection | Server path |
| --- | --- |
| `C:\Users\Yannick\Desktop` | `/Desktop` |
| `C:\Users\Yannick\Documents\Büro` | `/Documents/Büro` |
| `/home/user/Documents/Office` | `/Documents/Office` |
| `D:\Projects` (outside the home directory) | `/Projects` |

Missing server ancestors are created as directories. Only the selected leaf's tree is synchronized: selecting `Documents/Büro` does not synchronize other folders under `Documents`. Server paths are displayed, not editable. Filesystem roots, the entire home directory, reserved names, overlapping local trees and overlapping server paths are rejected. Names must be portable to Windows and Linux. Two outside-home folders with the same name cannot share a profile.

**Stop sync** removes the selection without deleting local files, remote files or recovery history. No remote delete operation is performed by removing a selection. A stopped local folder keeps its `.filenode` state. Re-adding it to the same account and path resumes its existing three-way baseline; moving it to a different account or server path is refused by the store identity checks.

Each selection has its own watcher, SQLite state, settling observations and retry deadline. An unavailable local drive is deferred without recreating its root; other selections continue. Failures retry with backoff from 5 seconds to 5 minutes. **Check now** retries all selections immediately. A selected server folder that was previously pinned by ID must not silently be recreated if it disappears.

Credentials, the server connection and upload/download budgets are shared by the account. Transfers run serially; adding folders does not multiply the rate limit. Power saver, sleep and Pause apply to all selections.

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
