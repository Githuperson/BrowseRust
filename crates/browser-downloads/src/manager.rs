use crate::fetch::{RequestContext, copy_body, follow};
use browser_security::{DownloadRisk, assess_download};
use parking_lot::Mutex;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobState {
    Running,
    Done,
    Failed,
    Cancelled,
}

struct Job {
    id: u64,
    url: String,
    filename: String,
    dest: Option<PathBuf>,
    state: JobState,
    received: u64,
    total: Option<u64>,
    risk: DownloadRisk,
    started: Instant,
    finished: Option<Instant>,
    error: Option<String>,
    cancel: Arc<AtomicBool>,
}

/// Read-only snapshot for the UI.
#[derive(Clone, Debug)]
pub struct JobView {
    pub id: u64,
    pub url: String,
    pub filename: String,
    pub dest: Option<PathBuf>,
    pub state: JobState,
    pub received: u64,
    pub total: Option<u64>,
    pub risk: DownloadRisk,
    pub bytes_per_sec: f64,
    pub error: Option<String>,
}

pub struct DownloadManager {
    dir: PathBuf,
    jobs: Arc<Mutex<Vec<Job>>>,
    next: AtomicU64,
}

/// `name.ext` -> `name (1).ext` ... first one that exists neither as a file nor a partial.
fn unique_path(dir: &Path, filename: &str) -> PathBuf {
    let p = Path::new(filename);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or("download");
    let ext = p.extension().and_then(|s| s.to_str());
    let taken = |c: &Path| c.exists() || PathBuf::from(format!("{}.part", c.display())).exists();
    let first = dir.join(filename);
    if !taken(&first) {
        return first;
    }
    for i in 1.. {
        let name = match ext {
            Some(e) => format!("{stem} ({i}).{e}"),
            None => format!("{stem} ({i})"),
        };
        let c = dir.join(name);
        if !taken(&c) {
            return c;
        }
    }
    unreachable!()
}

impl DownloadManager {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, jobs: Arc::new(Mutex::new(Vec::new())), next: AtomicU64::new(1) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Start downloading in a background thread. `filename` must already be sanitised.
    /// `cookies` resolves cookies for redirect targets (never reused across origins).
    pub fn start(&self, ctx: RequestContext, filename: String, total: Option<u64>, cookies: Arc<dyn Fn(&url::Url) -> Option<String> + Send + Sync>) -> u64 {
        let id = self.next.fetch_add(1, SeqCst);
        let cancel = Arc::new(AtomicBool::new(false));
        let risk = assess_download(&filename);
        self.jobs.lock().push(Job {
            id,
            url: ctx.url.to_string(),
            filename: filename.clone(),
            dest: None,
            state: JobState::Running,
            received: 0,
            total,
            risk,
            started: Instant::now(),
            finished: None,
            error: None,
            cancel: cancel.clone(),
        });
        let (jobs, dir) = (self.jobs.clone(), self.dir.clone());
        std::thread::spawn(move || {
            let set = |f: &mut dyn FnMut(&mut Job)| {
                if let Some(j) = jobs.lock().iter_mut().find(|j| j.id == id) {
                    f(j);
                }
            };
            let fail = |msg: String| {
                set(&mut |j| {
                    j.state = JobState::Failed;
                    j.error = Some(msg.clone());
                    j.finished = Some(Instant::now());
                })
            };
            if let Err(e) = std::fs::create_dir_all(&dir) {
                return fail(format!("cannot create downloads folder: {e}"));
            }
            let (resp, _final_url) = match follow(&ctx, &*cookies, false) {
                Ok(r) => r,
                Err(e) => return fail(e),
            };
            let status = resp.status().as_u16();
            if !(200..300).contains(&status) {
                return fail(format!("server returned HTTP {status}"));
            }
            let len = resp.headers().get("content-length").and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
            set(&mut |j| j.total = len.or(j.total));

            // Reserve a unique name without ever overwriting an existing file.
            let (dest, part, mut file) = loop {
                let dest = unique_path(&dir, &filename);
                let part = PathBuf::from(format!("{}.part", dest.display()));
                match std::fs::OpenOptions::new().write(true).create_new(true).open(&part) {
                    Ok(f) => break (dest, part, f),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => return fail(format!("cannot create file: {e}")),
                }
            };
            set(&mut |j| j.dest = Some(dest.clone()));
            let result = copy_body(resp, &mut file, |n| set(&mut |j| j.received = n), || cancel.load(SeqCst));
            drop(file);
            match result {
                Ok(true) => match std::fs::rename(&part, &dest) {
                    Ok(()) => set(&mut |j| {
                        j.state = JobState::Done;
                        j.finished = Some(Instant::now());
                    }),
                    Err(e) => {
                        let _ = std::fs::remove_file(&part);
                        fail(format!("cannot finish file: {e}"))
                    }
                },
                Ok(false) => {
                    let _ = std::fs::remove_file(&part);
                    set(&mut |j| {
                        j.state = JobState::Cancelled;
                        j.finished = Some(Instant::now());
                        j.dest = None;
                    });
                }
                Err(e) => {
                    let _ = std::fs::remove_file(&part);
                    fail(e.to_string())
                }
            }
        });
        id
    }

    pub fn cancel(&self, id: u64) {
        if let Some(j) = self.jobs.lock().iter().find(|j| j.id == id) {
            j.cancel.store(true, SeqCst);
        }
    }

    pub fn clear_finished(&self) {
        self.jobs.lock().retain(|j| j.state == JobState::Running);
    }

    pub fn active_count(&self) -> usize {
        self.jobs.lock().iter().filter(|j| j.state == JobState::Running).count()
    }

    pub fn snapshot(&self) -> Vec<JobView> {
        self.jobs
            .lock()
            .iter()
            .map(|j| {
                let secs = j.finished.unwrap_or_else(Instant::now).duration_since(j.started).as_secs_f64().max(0.001);
                JobView {
                    id: j.id,
                    url: j.url.clone(),
                    filename: j.filename.clone(),
                    dest: j.dest.clone(),
                    state: j.state,
                    received: j.received,
                    total: j.total,
                    risk: j.risk,
                    bytes_per_sec: j.received as f64 / secs,
                    error: j.error.clone(),
                }
            })
            .collect()
    }
}
