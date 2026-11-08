//! Account-free native diagnostics. Never log protocol commands, credentials,
//! HTTP headers/bodies, browser callbacks or per-file transfer traffic.
use std::{
    collections::HashMap,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex, OnceLock,
    },
    time::{Duration, Instant},
};

static ENABLED: AtomicBool = AtomicBool::new(false);
type Recent = HashMap<(u32, String), (String, Instant)>;
static RECENT: OnceLock<Mutex<Recent>> = OnceLock::new();

#[derive(Clone, Copy)]
pub enum Level {
    Information,
    Warning,
    Error,
}

pub fn init() {
    ENABLED.store(
        std::env::var_os("FILENODE_LOG").is_none_or(|v| v != "off"),
        Ordering::Relaxed,
    );
}

/// Repeated identical failures for a folder are limited to one entry per minute.
/// A different cause, a different folder or recovery is logged immediately.
pub fn event(level: Level, id: u32, name: &str, folder: Option<&str>, details: &str) {
    if !ENABLED.load(Ordering::Relaxed) {
        return;
    }
    let message = message(name, folder, details);
    let mut recent = RECENT
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let key = (id, folder.unwrap_or("").to_owned());
    if recent.get(&key).is_some_and(|(previous, when)| {
        previous == &message && when.elapsed() < Duration::from_secs(60)
    }) {
        return;
    }
    if recent.len() >= 128 {
        recent.clear();
    }
    recent.insert(key, (message.clone(), Instant::now()));
    drop(recent);
    if let Err(error) = write_native(level, id, &message) {
        // Keep diagnostics out of the desktop's JSON stdout channel. A broken
        // or unavailable logging daemon must never stop synchronization.
        eprintln!("FileNode native logging unavailable ({error}): {message}");
    }
}

pub fn self_test() -> io::Result<()> {
    write_native(
        Level::Information,
        999,
        &message(
            "logging_test",
            None,
            "Native logging test; no account or files accessed",
        ),
    )
}

fn message(name: &str, folder: Option<&str>, details: &str) -> String {
    let folder = folder
        .map(|p| format!(" folder={}", clean(p)))
        .unwrap_or_default();
    format!(
        "FileNode event={} pid={}{}: {}",
        clean(name),
        std::process::id(),
        folder,
        clean(details)
    )
}

fn clean(value: &str) -> String {
    let mut value: String = value
        .chars()
        .take(8192)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let lower = value.to_ascii_lowercase();
    if [
        "authorization:",
        "authorization=",
        "password=",
        "\"password\"",
        "access_token=",
        "\"access_token\"",
        "refresh_token=",
        "\"refresh_token\"",
        "client_secret=",
        "\"client_secret\"",
    ]
    .iter()
    .any(|key| lower.contains(key))
    {
        return "[sensitive diagnostic omitted]".into();
    }
    // Reqwest errors can contain complete URLs, including query credentials.
    // Remove the entire URL rather than trying to identify individual secrets.
    loop {
        let lower = value.to_ascii_lowercase();
        let start = [lower.find("https://"), lower.find("http://")]
            .into_iter()
            .flatten()
            .min();
        let Some(start) = start else {
            break;
        };
        let end = value[start..]
            .find(|c: char| c.is_whitespace() || matches!(c, '\'' | '"' | '<' | '>'))
            .map_or(value.len(), |n| start + n);
        value.replace_range(start..end, "[URL omitted]");
    }
    value
}

#[cfg(target_os = "linux")]
fn write_native(level: Level, id: u32, message: &str) -> io::Result<()> {
    write_linux(
        level,
        id,
        message,
        std::path::Path::new("/run/systemd/journal/socket"),
        std::path::Path::new("/dev/log"),
    )
}

#[cfg(target_os = "linux")]
fn write_linux(
    level: Level,
    id: u32,
    message: &str,
    journal: &std::path::Path,
    syslog: &std::path::Path,
) -> io::Result<()> {
    use std::os::unix::net::UnixDatagram;
    let priority = match level {
        Level::Information => 6,
        Level::Warning => 4,
        Level::Error => 3,
    };
    let socket = UnixDatagram::unbound()?;
    socket.set_nonblocking(true)?;
    let fields = format!("SYSLOG_IDENTIFIER=filenode\nSYSLOG_FACILITY=1\nPRIORITY={priority}\nFILENODE_EVENT_ID={id}\nMESSAGE={message}\n");
    if socket.send_to(fields.as_bytes(), journal).is_ok() {
        return Ok(());
    }
    let timestamp = chrono::Local::now().format("%b %e %T");
    let packet = format!(
        "<{}>{timestamp} filenode[{}]: {message}",
        8 + priority,
        std::process::id()
    );
    socket.send_to(packet.as_bytes(), syslog).map(|_| ())
}

#[cfg(target_os = "windows")]
fn write_native(level: Level, id: u32, message: &str) -> io::Result<()> {
    use std::{ffi::c_void, ptr};
    #[link(name = "advapi32")]
    unsafe extern "system" {
        fn RegisterEventSourceW(server: *const u16, source: *const u16) -> *mut c_void;
        fn DeregisterEventSource(handle: *mut c_void) -> i32;
        fn ReportEventW(
            handle: *mut c_void,
            kind: u16,
            category: u16,
            id: u32,
            sid: *const c_void,
            count: u16,
            size: u32,
            strings: *const *const u16,
            data: *const c_void,
        ) -> i32;
    }
    let source: Vec<u16> = "FileNode".encode_utf16().chain(Some(0)).collect();
    let text: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();
    let kind = match level {
        Level::Information => 4,
        Level::Warning => 2,
        Level::Error => 1,
    };
    // All UTF-16 buffers and the insertion pointer remain live for the call.
    unsafe {
        let handle = RegisterEventSourceW(ptr::null(), source.as_ptr());
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        let result = ReportEventW(
            handle,
            kind,
            0,
            windows_event_identifier(id),
            ptr::null(),
            1,
            0,
            &text.as_ptr(),
            ptr::null(),
        );
        let error = if result == 0 {
            Some(io::Error::last_os_error())
        } else {
            None
        };
        DeregisterEventSource(handle);
        match error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_event_identifier(id: u32) -> u32 {
    static REGISTERED: OnceLock<bool> = OnceLock::new();
    let registered = *REGISTERED.get_or_init(|| {
        use std::ffi::c_void;
        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn RegGetValueW(
                key: isize,
                subkey: *const u16,
                value: *const u16,
                flags: u32,
                kind: *mut u32,
                data: *mut c_void,
                size: *mut u32,
            ) -> i32;
        }
        let key: Vec<u16> = "SYSTEM\\CurrentControlSet\\Services\\EventLog\\Application\\FileNode"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let name: Vec<u16> = "EventMessageFile".encode_utf16().chain(Some(0)).collect();
        let mut buffer = [0u16; 1024];
        let mut size = std::mem::size_of_val(&buffer) as u32;
        // Read and expand the registration, without creating a registry key or UAC.
        let result = unsafe {
            RegGetValueW(
                0x80000002u32 as i32 as isize,
                key.as_ptr(),
                name.as_ptr(),
                0x6,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        let end = buffer.iter().position(|c| *c == 0).unwrap_or(buffer.len());
        let expected = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into())
            + "\\System32\\EventCreate.exe";
        result == 0 && String::from_utf16_lossy(&buffer[..end]).eq_ignore_ascii_case(&expected)
    });
    // Without a registered message resource, do not let a viewer interpret our
    // event 310 as Win32 ERROR_INVALID_EXCEPTION_HANDLER (also 310). The customer
    // bit separates these identifiers; the low event ID and insertion text remain.
    if registered {
        id
    } else {
        id | 0x2000_0000
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
fn write_native(_: Level, _: u32, message: &str) -> io::Result<()> {
    eprintln!("{message}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diagnostics_remove_urls_secrets_and_forged_log_fields() {
        assert_eq!(
            clean("failed (https://user:secret@example.org/api?code=private)"),
            "failed ([URL omitted]"
        );
        assert_eq!(
            clean("HTTP://example.org/?access_token=secret"),
            "[sensitive diagnostic omitted]"
        );
        assert_eq!(
            clean("{\"refresh_token\":\"private\"}"),
            "[sensitive diagnostic omitted]"
        );
        assert_eq!(clean("failure\nPRIORITY=0\0"), "failure PRIORITY=0 ");
        assert_eq!(clean(&"ä".repeat(9000)).chars().count(), 8192);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn native_journal_fields_and_syslog_fallback_reach_local_sockets() {
        use std::os::unix::net::UnixDatagram;
        let directory = tempfile::tempdir().unwrap();
        let journal = directory.path().join("journal");
        let syslog = directory.path().join("syslog");
        let receive = UnixDatagram::bind(&journal).unwrap();
        receive
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        let mut buffer = [0; 4096];
        write_linux(
            Level::Error,
            300,
            "Folder unavailable · Büro",
            &journal,
            &syslog,
        )
        .unwrap();
        let count = receive.recv(&mut buffer).unwrap();
        let entry = std::str::from_utf8(&buffer[..count]).unwrap();
        assert!(entry.contains("PRIORITY=3\n"));
        assert!(entry.contains("SYSLOG_IDENTIFIER=filenode\n"));
        assert!(entry.contains("FILENODE_EVENT_ID=300\n"));
        assert!(entry.contains("MESSAGE=Folder unavailable · Büro\n"));
        let fallback = UnixDatagram::bind(&syslog).unwrap();
        fallback
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        write_linux(
            Level::Warning,
            200,
            "Retry scheduled",
            &directory.path().join("missing"),
            &syslog,
        )
        .unwrap();
        let count = fallback.recv(&mut buffer).unwrap();
        let entry = std::str::from_utf8(&buffer[..count]).unwrap();
        assert!(entry.starts_with("<12>"));
        assert!(entry.ends_with("Retry scheduled"));
        assert!(write_linux(
            Level::Error,
            300,
            "Unavailable",
            &directory.path().join("missing"),
            &directory.path().join("also-missing")
        )
        .is_err());
    }
}
