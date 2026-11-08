//! Structured desktop events. Byte progress never marks a file committed;
//! only the engine's successful baseline update can finish an activity.
use serde::Serialize;
use std::{
    cell::RefCell,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    Renamed {
        folder: String,
        from: String,
        to: String,
    },
    File {
        folder: String,
        path: String,
        operation: String,
        state: String,
        completed: u64,
        total: u64,
    },
    Folder {
        folder: String,
        state: String,
        retry_seconds: u64,
        details: String,
    },
}
type Sink = Arc<dyn Fn(Event) + Send + Sync>;
pub fn renamed(from: &str, to: &str) {
    CURRENT.with(|current| {
        let current = current.borrow();
        if let Some(sink) = &current.sink {
            sink(Event::Renamed {
                folder: current.folder.clone(),
                from: from.into(),
                to: to.into(),
            });
        }
    });
}
#[derive(Clone, Default)]
struct Context {
    sink: Option<Sink>,
    folder: String,
    transfer: Option<Arc<Transfer>>,
    notices: usize,
}
thread_local! { static CURRENT: RefCell<Context> = RefCell::new(Context::default()); }
pub struct Scope(Context);
impl Scope {
    pub fn observe(sink: impl Fn(Event) + Send + Sync + 'static) -> Self {
        Self(CURRENT.with(|current| {
            current.replace(Context {
                sink: Some(Arc::new(sink)),
                ..Context::default()
            })
        }))
    }
    pub fn folder(folder: String) -> Self {
        Self(CURRENT.with(|current| {
            let mut next = current.borrow().clone();
            next.folder = folder;
            next.transfer = None;
            next.notices = 0;
            current.replace(next)
        }))
    }
}
pub fn notice(path: &str, operation: &str, state: &str) {
    // The desktop retains 100 activities. A large settling scan must not flood IPC.
    CURRENT.with(|current| {
        let mut current = current.borrow_mut();
        if current.notices >= 100 {
            return;
        }
        current.notices += 1;
        if let Some(sink) = &current.sink {
            sink(Event::File {
                folder: current.folder.clone(),
                path: path.into(),
                operation: operation.into(),
                state: state.into(),
                completed: 0,
                total: 0,
            });
        }
    });
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.0.clone());
        });
    }
}
pub fn folder_status(state: &str, retry_seconds: u64, details: String) {
    CURRENT.with(|current| {
        let current = current.borrow();
        if let Some(sink) = &current.sink {
            sink(Event::Folder {
                folder: current.folder.clone(),
                state: state.into(),
                retry_seconds,
                details,
            });
        }
    });
}
struct Transfer {
    sink: Sink,
    folder: String,
    path: String,
    operation: String,
    total: u64,
    completed: AtomicU64,
    closed: AtomicBool,
    last: Mutex<Instant>,
}
impl Transfer {
    fn emit(&self, state: &str) {
        (self.sink)(Event::File {
            folder: self.folder.clone(),
            path: self.path.clone(),
            operation: self.operation.clone(),
            state: state.into(),
            completed: self.completed.load(Ordering::Relaxed),
            total: self.total,
        });
    }
}
#[derive(Clone)]
pub struct Progress(Option<Arc<Transfer>>);
impl Progress {
    pub fn current() -> Self {
        Self(CURRENT.with(|current| current.borrow().transfer.clone()))
    }
    pub fn set(&self, completed: u64) {
        if let Some(transfer) = &self.0 {
            if transfer.closed.load(Ordering::Relaxed) {
                return;
            }
            let mut last = transfer.last.lock().unwrap_or_else(|e| e.into_inner());
            if transfer.closed.load(Ordering::Relaxed) {
                return;
            }
            transfer
                .completed
                .store(completed.min(transfer.total), Ordering::Relaxed);
            if last.elapsed() >= Duration::from_millis(120) || completed >= transfer.total {
                *last = Instant::now();
                transfer.emit("transferring");
            }
        }
    }
}
pub struct FileActivity {
    previous: Option<Arc<Transfer>>,
    transfer: Option<Arc<Transfer>>,
    finished: bool,
}
impl FileActivity {
    pub fn begin(path: &str, operation: &str, total: u64) -> Self {
        let (previous, transfer) = CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            let transfer = current.sink.as_ref().map(|sink| {
                Arc::new(Transfer {
                    sink: sink.clone(),
                    folder: current.folder.clone(),
                    path: path.into(),
                    operation: operation.into(),
                    total,
                    completed: AtomicU64::new(0),
                    closed: AtomicBool::new(false),
                    last: Mutex::new(Instant::now()),
                })
            });
            let previous = std::mem::replace(&mut current.transfer, transfer.clone());
            (previous, transfer)
        });
        if let Some(transfer) = &transfer {
            transfer.emit("transferring");
        }
        Self {
            previous,
            transfer,
            finished: false,
        }
    }
    pub fn finish(&mut self, state: &str) {
        self.finished = true;
        if let Some(transfer) = &self.transfer {
            let _lock = transfer.last.lock().unwrap_or_else(|e| e.into_inner());
            transfer.closed.store(true, Ordering::Relaxed);
            if state == "synced" {
                transfer.completed.store(transfer.total, Ordering::Relaxed);
            }
            transfer.emit(state);
        }
    }
}
impl Drop for FileActivity {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(transfer) = &self.transfer {
                let _lock = transfer.last.lock().unwrap_or_else(|e| e.into_inner());
                transfer.closed.store(true, Ordering::Relaxed);
                transfer.emit(if crate::runtime::check().is_err() {
                    "paused"
                } else {
                    "retrying"
                });
            }
        }
        CURRENT.with(|current| current.borrow_mut().transfer = self.previous.take());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn full_payload_is_not_success_until_commit_and_failure_retries_the_same_identity() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let _observer = Scope::observe(move |event| sink.lock().unwrap().push(event));
        let _folder = Scope::folder("/home/user/Desktop".into());
        {
            let _file = FileActivity::begin("a.txt", "upload", 100);
            Progress::current().set(100);
        }
        {
            let mut file = FileActivity::begin("a.txt", "upload", 100);
            let progress = Progress::current();
            progress.set(100);
            file.finish("synced");
            // A late HTTP body callback must not resurrect a completed row.
            progress.set(20);
        }
        let events = events.lock().unwrap();
        let states: Vec<_> = events
            .iter()
            .map(|event| match event {
                Event::File {
                    state,
                    folder,
                    path,
                    ..
                } => {
                    assert_eq!(folder, "/home/user/Desktop");
                    assert_eq!(path, "a.txt");
                    state.as_str()
                }
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            states,
            [
                "transferring",
                "transferring",
                "retrying",
                "transferring",
                "transferring",
                "synced"
            ]
        );
    }
    #[test]
    fn settling_notifications_are_bounded_per_folder_without_reporting_transfers() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let _observer = Scope::observe(move |event| sink.lock().unwrap().push(event));
        let _folder = Scope::folder("/home/user/Desktop".into());
        for index in 0..1000 {
            notice(&format!("{index}.txt"), "check", "waiting");
        }
        let events = events.lock().unwrap();
        assert_eq!(events.len(), 100);
        assert!(events
            .iter()
            .all(|event| matches!(event, Event::File { state, .. } if state == "waiting")));
    }
}
