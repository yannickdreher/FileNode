# Native diagnostics

The Rust worker logs account verification, synchronization start/stop, connection failures, per-folder failures with retry delay, and recovery. Routine scans and file-progress events are not logged. Identical entries are limited to once per minute per event ID and folder. The queue-free local native transport is best effort: missing or unavailable logging facilities never stop synchronization, and the sanitized diagnostic is sent to stderr instead. Desktop JSON stdout remains exclusively the IPC channel.

Diagnostics include the local folder path and cause, so they may contain local usernames and filenames. They exclude credentials, configuration payloads, HTTP bodies/headers and authorization callbacks. Complete HTTP URLs are removed, common credential-field diagnostics are omitted, control characters are flattened, and text is bounded. System administrators control access and retention in the operating system log.

## Windows

Open **Event Viewer → Windows Logs → Application** and filter by source **FileNode**. Severity is Information, Warning or Error. The message includes the process ID, event name, folder when applicable, and cause.

The application uses `RegisterEventSourceW` / `ReportEventW` without requesting administrator rights. A portable build can write under its unregistered source; Windows then shows the insertion text in the event details but may report a missing description resource. Unregistered events set the customer bit, keeping FileNode event identifiers separate from unrelated Win32 errors with the same number. The displayed low event ID stays the same; XML qualifiers include 8192. Older unregistered event 310 may misleadingly display the Windows system text “An invalid exception handler routine has been detected”; inspect its EventData for the actual FileNode cause. For friendly descriptions, run the following once in **Windows PowerShell as administrator**:

```powershell
.\scripts\register-eventlog.ps1
```

This registers the built-in Windows `EventCreate.exe` message resource, supporting IDs 1–1000, rather than a DLL in a user-writable app directory. It changes only the FileNode source and refuses to overwrite an existing registration. Application startup never changes the machine registry or triggers UAC. The script is intended for portable/developer installations; Store packaging does not perform this machine-wide registration.

```powershell
.\.dev\filenode-preview\filenode-core.exe --test-log
Get-WinEvent -LogName Application -FilterXPath "*[System[Provider[@Name='FileNode']]]" -MaxEvents 20 |
    Select-Object TimeCreated, Id, LevelDisplayName, Message,
        @{ Name='Details'; Expression={ $_.Properties.Value -join ' | ' } }
```

## Linux

The worker sends native structured datagrams to `/run/systemd/journal/socket`, with `SYSLOG_IDENTIFIER=filenode`, severity and `FILENODE_EVENT_ID`. If that fails, it falls back to the local `/dev/log` syslog socket. Datagram sends are nonblocking; no daemon or libsystemd dependency is added.

```sh
journalctl -t filenode --since today
journalctl -t filenode -f
./filenode-core --test-log
```

On a system without journald, inspect the configured syslog destination. Containers may provide neither socket; `--test-log` then returns an error instead of claiming successful delivery. Normal synchronization continues with stderr diagnostics.

## Event IDs

| ID | Meaning |
| --- | --- |
| 110 | File-account access verified |
| 120 | Sync started/stopped |
| 200 | Connection retry |
| 210 | Folder recovered |
| 220 | Push unavailable; reconnecting with polling fallback |
| 300 | Folder failure and retry delay |
| 310 | Worker/configuration/authentication error |
| 398 | Unhandled Windows UI exception (type, HRESULT and method names; no exception message/arguments) |
| 399 | Fatal process error |
| 999 | Explicit account-free logging test |

Folder cards also expose their current error in selectable technical details. A verified account and a folder failure are independent: a local access error, unsupported local path, size limit or later server operation can fail after successful sign-in.

Protocol and UI smoke tests set `FILENODE_LOG=off` for their worker processes so intentional validation/connection failures do not pollute the host log. The same environment setting can disable routine native logging; an explicit `--test-log` still writes its requested diagnostic entry.

References: [Microsoft Event Sources](https://learn.microsoft.com/en-us/windows/win32/eventlog/event-sources), [ReportEventW](https://learn.microsoft.com/en-us/windows/win32/api/winbase/nf-winbase-reporteventw), [Go event-source registration with the Windows message resource](https://go.googlesource.com/sys/+/master/windows/svc/eventlog/install.go), [systemd native journal protocol](https://systemd.io/JOURNAL_NATIVE_PROTOCOL/).
