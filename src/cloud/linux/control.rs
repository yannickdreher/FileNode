//! Local file-manager bridge. No credentials or arbitrary filesystem operations.
use super::*;
use std::{
    io::{BufRead, BufReader},
    os::{
        fd::AsRawFd,
        unix::{
            fs::PermissionsExt,
            net::{UnixListener, UnixStream},
        },
    },
    thread::JoinHandle,
    time::Duration,
};

fn directory() -> Result<PathBuf> {
    let parent = std::env::var_os("XDG_RUNTIME_DIR")
        .context("XDG_RUNTIME_DIR is required for Linux cloud integration")?;
    let directory = PathBuf::from(parent).join("filenode");
    crate::store::ensure_state_directory(&directory)?;
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    Ok(directory)
}
pub(super) struct Server {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    path: PathBuf,
}
impl Server {
    pub fn start(state: &Arc<State>) -> Result<Self> {
        let path = directory()?.join(format!("{}.sock", state.key));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                bail!("Another process owns this cloud mount");
            }
            fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let weak = Arc::downgrade(state);
        let thread = std::thread::Builder::new()
            .name("cloud-file-manager".into())
            .spawn(move || {
                let mut jobs: Vec<JoinHandle<()>> = Vec::new();
                while !stopping.load(Ordering::SeqCst) {
                    for i in (0..jobs.len()).rev() {
                        if jobs[i].is_finished() {
                            let _ = jobs.swap_remove(i).join();
                        }
                    }
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            if let Some(state) = weak.upgrade() {
                                let response = serve(&state, &mut stream, &mut jobs)
                                    .unwrap_or_else(|_| serde_json::json!({"ok": false}));
                                let _ = writeln!(stream, "{response}");
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            std::thread::sleep(Duration::from_millis(30))
                        }
                        Err(_) => break,
                    }
                }
                for job in jobs {
                    let _ = job.join();
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
            path,
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}
fn serve(
    state: &Arc<State>,
    stream: &mut UnixStream,
    jobs: &mut Vec<JoinHandle<()>>,
) -> Result<serde_json::Value> {
    let mut credentials = std::mem::MaybeUninit::<libc::ucred>::uninit();
    let mut length = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            credentials.as_mut_ptr().cast(),
            &mut length,
        )
    } != 0
        || unsafe { credentials.assume_init().uid != libc::geteuid() }
    {
        bail!("Cloud command came from another user");
    }
    stream.set_read_timeout(Some(Duration::from_millis(500)))?;
    stream.set_write_timeout(Some(Duration::from_millis(500)))?;
    let mut line = String::new();
    BufReader::new((&mut *stream).take(32 * 1024)).read_line(&mut line)?;
    let request: serde_json::Value = serde_json::from_str(&line)?;
    let absolute = Path::new(request["path"].as_str().context("Missing path")?);
    let relative = absolute.strip_prefix(&state.root)?;
    if !relative
        .components()
        .all(|c| matches!(c, std::path::Component::Normal(_)))
    {
        bail!("Invalid cloud path");
    }
    let relative = relative
        .to_str()
        .context("Non-UTF-8 cloud path")?
        .to_owned();
    let entry = state
        .catalog
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .path(&relative)?
        .context("Path is not in this mount")?;
    match request["action"].as_str() {
        Some("status") => Ok(
            serde_json::json!({"ok":true,"state":entry.status(),"root":state.root,"pinned":entry.pinned,"size":entry.size}),
        ),
        Some(action @ ("pin" | "evict")) => {
            if jobs.len() >= 4 {
                bail!("Cloud command queue is full");
            }
            let state = state.clone();
            let pin = action == "pin";
            jobs.push(std::thread::spawn(move || {
                if let Err(error) = state.availability(&relative, pin) {
                    crate::logger::event(
                        crate::logger::Level::Warning,
                        300,
                        "cloud_availability_retry",
                        Some(&state.root.to_string_lossy()),
                        &format!("{error:#}"),
                    );
                }
            }));
            Ok(serde_json::json!({"ok":true,"accepted":true}))
        }
        _ => bail!("Unknown cloud action"),
    }
}

pub fn command(action: &str, path: &Path) -> Result<serde_json::Value> {
    if !["status", "pin", "evict"].contains(&action) {
        bail!("Unknown cloud action");
    }
    let path = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    for candidate in fs::read_dir(directory()?)? {
        let candidate = candidate?.path();
        if candidate.extension().is_none_or(|e| e != "sock") {
            continue;
        }
        let Ok(mut stream) = UnixStream::connect(candidate) else {
            continue;
        };
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        writeln!(
            stream,
            "{}",
            serde_json::json!({"action":action,"path":path})
        )?;
        let mut line = String::new();
        BufReader::new(stream.take(32 * 1024)).read_line(&mut line)?;
        let response: serde_json::Value = serde_json::from_str(&line)?;
        if response["ok"] == true {
            return Ok(response);
        }
    }
    bail!("No running FileNode cloud mount contains this path")
}
