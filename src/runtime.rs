//! Shared transfer budgets and cooperative interruption at I/O boundaries.
use crate::config::Config;
use std::{
    cell::RefCell,
    io::{self, Read},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct PowerState {
    pub saver: AtomicBool,
    pub sleeping: AtomicBool,
    pub available: AtomicBool,
    pub revision: AtomicU64,
}

impl PowerState {
    pub fn set(&self, saver: bool, sleeping: bool) {
        let changed = self.saver.swap(saver, Ordering::SeqCst) != saver;
        let changed = self.sleeping.swap(sleeping, Ordering::SeqCst) != sleeping || changed;
        if changed {
            self.revision.fetch_add(1, Ordering::SeqCst);
        }
    }
}

pub struct Policy {
    upload_schedule: Mutex<Schedule>,
    download_schedule: Mutex<Schedule>,
    pub cancel: Arc<AtomicBool>,
    pub power: Arc<PowerState>,
    pub pause_on_saver: AtomicBool,
    pub upload: AtomicU64,
    pub download: AtomicU64,
}

impl Policy {
    pub fn new(config: &Config, cancel: Arc<AtomicBool>, power: Arc<PowerState>) -> Arc<Self> {
        let policy = Arc::new(Self {
            upload_schedule: Mutex::new(Schedule {
                next: Instant::now(),
                rate: 0,
                revision: 0,
            }),
            download_schedule: Mutex::new(Schedule {
                next: Instant::now(),
                rate: 0,
                revision: 0,
            }),
            cancel,
            power,
            pause_on_saver: AtomicBool::new(true),
            upload: AtomicU64::new(0),
            download: AtomicU64::new(0),
        });
        policy.configure(config);
        policy
    }
    pub fn configure(&self, config: &Config) {
        self.upload.store(
            config.upload_limit_kib.saturating_mul(1024),
            Ordering::Relaxed,
        );
        self.download.store(
            config.download_limit_kib.saturating_mul(1024),
            Ordering::Relaxed,
        );
        self.pause_on_saver
            .store(config.pause_on_power_saver, Ordering::Relaxed);
    }
    pub fn power_paused(&self) -> bool {
        self.power.sleeping.load(Ordering::SeqCst)
            || (self.pause_on_saver.load(Ordering::Relaxed)
                && self.power.saver.load(Ordering::SeqCst))
    }
    pub fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) || self.power_paused() {
            Err(io::Error::other("Operation paused by user or power policy"))
        } else {
            Ok(())
        }
    }
}

thread_local! { static CURRENT: RefCell<Option<Arc<Policy>>> = const { RefCell::new(None) }; }
pub struct Scope(Option<Arc<Policy>>);
impl Scope {
    pub fn enter(policy: Arc<Policy>) -> Self {
        Self(CURRENT.with(|current| current.replace(Some(policy))))
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        CURRENT.with(|current| {
            current.replace(self.0.take());
        });
    }
}
pub fn check() -> io::Result<()> {
    CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .map_or(Ok(()), |policy| policy.check())
    })
}

/// Payload pacing. Deliberately no initial burst; buffers are bounded to 16 KiB.
/// Socket/TLS buffers and protocol overhead are outside this payload budget.
pub struct Budget {
    policy: Arc<Policy>,
    upload: bool,
}
struct Schedule {
    next: Instant,
    rate: u64,
    revision: u64,
}
impl Budget {
    pub fn new(policy: Arc<Policy>, upload: bool) -> Self {
        Self { policy, upload }
    }
    pub fn reserve(&mut self, bytes: usize) -> io::Result<()> {
        let rate = if self.upload {
            &self.policy.upload
        } else {
            &self.policy.download
        };
        let value = rate.load(Ordering::Relaxed);
        let revision = self.policy.power.revision.load(Ordering::SeqCst);
        self.policy.check()?;
        if value == 0 {
            return Ok(());
        }
        let deadline = {
            let schedule = if self.upload {
                &self.policy.upload_schedule
            } else {
                &self.policy.download_schedule
            };
            let mut schedule = schedule.lock().unwrap_or_else(|e| e.into_inner());
            if schedule.rate != value || schedule.revision != revision {
                schedule.next = Instant::now();
                schedule.rate = value;
                schedule.revision = revision;
            }
            schedule.next = schedule.next.max(Instant::now())
                + Duration::from_secs_f64(bytes as f64 / value as f64);
            schedule.next
        };
        while Instant::now() < deadline {
            self.policy.check()?;
            if rate.load(Ordering::Relaxed) != value
                || self.policy.power.revision.load(Ordering::SeqCst) != revision
            {
                return self.reserve(bytes);
            }
            std::thread::sleep(
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_millis(50)),
            );
        }
        Ok(())
    }
}

pub struct LimitedReader<R> {
    source: R,
    budget: Budget,
    progress: crate::activity::Progress,
    completed: u64,
}
impl<R> LimitedReader<R> {
    pub fn new(source: R, policy: Arc<Policy>) -> Self {
        Self {
            source,
            budget: Budget::new(policy, true),
            progress: crate::activity::Progress::current(),
            completed: 0,
        }
    }
}
impl<R: Read> Read for LimitedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.budget.policy.check()?;
        let maximum = buffer.len().min(16 * 1024);
        let read = self.source.read(&mut buffer[..maximum])?;
        self.budget.reserve(read)?;
        self.completed += read as u64;
        self.progress.set(self.completed);
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resume_discards_unused_bandwidth_reservations_from_before_sleep() {
        let config = Config {
            upload_limit_kib: 100,
            ..Default::default()
        };
        let power = Arc::new(PowerState::default());
        let policy = Policy::new(&config, Arc::new(AtomicBool::new(false)), power.clone());
        let uploading = policy.clone();
        let worker = std::thread::spawn(move || Budget::new(uploading, true).reserve(1024 * 1024));
        std::thread::sleep(Duration::from_millis(50));
        power.set(false, true);
        assert!(worker.join().unwrap().is_err());
        power.set(false, false);
        let start = Instant::now();
        Budget::new(policy, true).reserve(10 * 1024).unwrap();
        assert!(start.elapsed() < Duration::from_secs(2));
    }
    #[test]
    fn concurrent_readers_share_the_account_payload_budget() {
        let config = Config {
            upload_limit_kib: 100,
            ..Default::default()
        };
        let policy = Policy::new(
            &config,
            Arc::new(AtomicBool::new(false)),
            Arc::new(PowerState::default()),
        );
        let start = Instant::now();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let policy = policy.clone();
                scope.spawn(move || Budget::new(policy, true).reserve(20 * 1024).unwrap());
            }
        });
        assert!(start.elapsed() >= Duration::from_millis(790));
    }
    #[test]
    fn budgets_delay_payload_and_power_pause_interrupts_work() {
        let power = Arc::new(PowerState::default());
        let config = Config {
            upload_limit_kib: 100,
            ..Default::default()
        };
        let policy = Policy::new(&config, Arc::new(AtomicBool::new(false)), power.clone());
        let start = Instant::now();
        Budget::new(policy.clone(), true)
            .reserve(20 * 1024)
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(190));
        power.set(true, false);
        assert!(policy.check().is_err());
        policy.pause_on_saver.store(false, Ordering::Relaxed);
        assert!(policy.check().is_ok());
        power.set(false, true);
        assert!(policy.check().is_err());
        power.set(false, false);
        assert!(policy.check().is_ok());
    }
}
