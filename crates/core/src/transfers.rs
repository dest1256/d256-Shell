//! Transfer queue: jobs run in the background, at most `parallel` at a time; the UI polls `list`.
use crate::remote_path;
use crate::sftp::xfer::{Action, Ctl, Outcome, XferError};
use crate::sftp::SftpConnection;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Upload,
    Download,
}

#[derive(Debug, Clone)]
pub struct Item {
    /// Local path (upload) or remote path (download).
    pub path: String,
    pub action: Action,
}

#[derive(Debug, Clone)]
pub struct JobParams {
    pub session_id: String,
    pub kind: Kind,
    pub items: Vec<Item>,
    /// Remote folder (upload) or local folder (download).
    pub dest: String,
    pub keep_mtime: bool,
    /// Refuse to start when more than this many bytes would be transferred (drag-out staging).
    pub max_bytes: Option<u64>,
    /// Upload risk flags chosen by the user after a warning: (replace foreign-owned file, write in place).
    pub risk: (bool, bool),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum State {
    Queued,
    Running,
    Done,
    Error { code: &'static str, message: String },
    Cancelled,
}

pub struct Job {
    id: String,
    label: String,
    params: JobParams,
    ctl: Arc<Ctl>,
    state: Mutex<State>,
    started: Mutex<Option<Instant>>,
    finished: Mutex<Option<Instant>>,
    outcome: Mutex<Outcome>,
    removed: AtomicBool,
}

pub struct Transfers {
    jobs: Mutex<Vec<Arc<Job>>>,
    running: Arc<AtomicUsize>,
    parallel: AtomicUsize,
    handle: tokio::runtime::Handle,
}

fn new_id() -> String {
    let mut b = [0u8; 6];
    getrandom::fill(&mut b).expect("rng");
    b.iter().map(|x| format!("{x:02x}")).collect()
}

impl Transfers {
    pub fn new(handle: tokio::runtime::Handle) -> Self {
        Self {
            jobs: Mutex::new(Vec::new()),
            running: Arc::new(AtomicUsize::new(0)),
            parallel: AtomicUsize::new(2),
            handle,
        }
    }

    pub fn set_parallel(&self, n: usize) {
        self.parallel.store(n.clamp(1, 8), Ordering::Relaxed);
    }

    pub fn start(self: &Arc<Self>, conn: Arc<SftpConnection>, params: JobParams) -> String {
        let id = new_id();
        let first = params
            .items
            .first()
            .map(|i| i.path.rsplit(['/', '\\']).next().unwrap_or("").to_string())
            .unwrap_or_default();
        let label = if params.items.len() > 1 {
            format!("{first} plus {}", params.items.len() - 1)
        } else {
            first
        };
        let job = Arc::new(Job {
            id: id.clone(),
            label,
            params,
            ctl: Arc::new(Ctl::default()),
            state: Mutex::new(State::Queued),
            started: Mutex::new(None),
            finished: Mutex::new(None),
            outcome: Mutex::new(Outcome::default()),
            removed: AtomicBool::new(false),
        });
        self.jobs.lock().expect("lock").push(job.clone());
        let this = self.clone();
        self.handle.spawn(async move {
            // wait for a free slot (cancel while queued ends the job at once)
            loop {
                if job.ctl.cancel.load(Ordering::Relaxed) {
                    *job.state.lock().expect("lock") = State::Cancelled;
                    return;
                }
                let n = this.running.load(Ordering::SeqCst);
                if n < this.parallel.load(Ordering::Relaxed)
                    && this
                        .running
                        .compare_exchange(n, n + 1, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            *job.state.lock().expect("lock") = State::Running;
            *job.started.lock().expect("lock") = Some(Instant::now());
            let res = run(&conn, &job).await;
            this.running.fetch_sub(1, Ordering::SeqCst);
            *job.finished.lock().expect("lock") = Some(Instant::now());
            *job.state.lock().expect("lock") = match res {
                Ok(()) => State::Done,
                Err(XferError::Cancelled) => State::Cancelled,
                Err(XferError::ForeignOwner) => State::Error {
                    code: "foreign_owner",
                    message: XferError::ForeignOwner.to_string(),
                },
                Err(XferError::NoPerm) => State::Error {
                    code: "failed",
                    message: XferError::NoPerm.to_string(),
                },
                Err(XferError::Exists) => State::Error {
                    code: "exists",
                    message: XferError::Exists.to_string(),
                },
                Err(XferError::Msg(m)) => State::Error {
                    code: if conn.is_closed() {
                        "connection_lost"
                    } else {
                        "failed"
                    },
                    message: m,
                },
            };
        });
        id
    }

    pub fn cancel(&self, id: &str) {
        if let Some(j) = self.find(id) {
            j.ctl.cancel.store(true, Ordering::Relaxed);
        }
    }

    pub fn cancel_session(&self, session_id: &str) {
        for j in self.jobs.lock().expect("lock").iter() {
            if j.params.session_id == session_id {
                j.ctl.cancel.store(true, Ordering::Relaxed);
            }
        }
    }

    fn find(&self, id: &str) -> Option<Arc<Job>> {
        self.jobs
            .lock()
            .expect("lock")
            .iter()
            .find(|j| j.id == id)
            .cloned()
    }

    /// Parameters of a finished job (for retry), with optional risk flags from the user's decision.
    pub fn retry_params(&self, id: &str, risk: Option<(bool, bool)>) -> Option<JobParams> {
        let j = self.find(id)?;
        if matches!(
            *j.state.lock().expect("lock"),
            State::Running | State::Queued
        ) {
            return None;
        }
        j.removed.store(true, Ordering::Relaxed);
        self.jobs.lock().expect("lock").retain(|x| x.id != id);
        let mut p = j.params.clone();
        if let Some(r) = risk {
            p.risk = r;
        }
        Some(p)
    }

    /// Drop finished jobs from the list.
    pub fn clear_finished(&self) {
        self.jobs.lock().expect("lock").retain(|j| {
            matches!(
                *j.state.lock().expect("lock"),
                State::Running | State::Queued
            )
        });
    }

    pub fn list(&self) -> Value {
        let jobs = self.jobs.lock().expect("lock");
        Value::Array(jobs.iter().map(|j| view(j)).collect())
    }

    pub fn view_of(&self, id: &str) -> Option<Value> {
        self.find(id).map(|j| view(&j))
    }
}

fn view(j: &Job) -> Value {
    let st = j.state.lock().expect("lock").clone();
    let (state, error_code, error) = match &st {
        State::Queued => ("queued", None, None),
        State::Running => ("running", None, None),
        State::Done => ("done", None, None),
        State::Cancelled => ("cancelled", None, None),
        State::Error { code, message } => ("error", Some(*code), Some(message.clone())),
    };
    let done = j.ctl.done.load(Ordering::Relaxed);
    let started = *j.started.lock().expect("lock");
    let finished = *j.finished.lock().expect("lock");
    let secs = match (started, finished) {
        (Some(s), Some(f)) => f.duration_since(s).as_secs_f64(),
        (Some(s), None) => s.elapsed().as_secs_f64(),
        _ => 0.0,
    };
    let speed = if secs > 0.2 {
        (done as f64 / secs) as u64
    } else {
        0
    };
    let out = j.outcome.lock().expect("lock").clone();
    json!({
        "id": j.id,
        "kind": match j.params.kind { Kind::Upload => "upload", Kind::Download => "download" },
        "label": j.label,
        "state": state,
        "done": done,
        "total": j.ctl.total.load(Ordering::Relaxed),
        "speed": speed,
        "current": j.ctl.current.lock().expect("lock").clone(),
        "error_code": error_code,
        "error": error,
        "files": out.files,
        "skipped": out.skipped,
        "dest": j.params.dest,
        "session_id": j.params.session_id,
        "item_paths": j.params.items.iter().map(|i| i.path.clone()).collect::<Vec<_>>(),
    })
}

async fn run(conn: &SftpConnection, job: &Job) -> Result<(), XferError> {
    let p = &job.params;
    conn.cleanup_stale().await;
    // total size first, so the progress bar is meaningful
    let mut total = 0u64;
    for it in &p.items {
        total += match p.kind {
            Kind::Upload => SftpConnection::local_totals(Path::new(&it.path)).0,
            Kind::Download => conn.remote_totals(&it.path, 0).await?.0,
        };
    }
    if let Some(max) = p.max_bytes {
        if total > max {
            return Err(XferError::Msg(format!(
                "Too much data to drag ({} MB, limit {} MB): use “Download…”.",
                total / (1024 * 1024),
                max / (1024 * 1024)
            )));
        }
    }
    job.ctl.total.store(total, Ordering::Relaxed);

    let mut out = Outcome::default();
    for it in &p.items {
        job.ctl
            .cancel
            .load(Ordering::Relaxed)
            .then_some(())
            .map_or(Ok(()), |_| Err(XferError::Cancelled))?;
        match p.kind {
            Kind::Upload => {
                let local = PathBuf::from(&it.path);
                let md = tokio::fs::metadata(&local)
                    .await
                    .map_err(|e| XferError::Msg(format!("{}: {e}", it.path)))?;
                let name = local
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or_else(|| XferError::Msg(format!("{}: invalid name", it.path)))?
                    .to_string();
                if md.is_dir() {
                    let mut action = it.action;
                    let mut dest_name = name.clone();
                    // a top-level name clash of folders merges unless the user chose "keep both"
                    if action == Action::Rename
                        && conn
                            .exists_lstat(&remote_path::join(&p.dest, &name))
                            .await?
                            .is_some()
                    {
                        dest_name = conn.free_numbered(&p.dest, &name).await?;
                        action = Action::Replace;
                    }
                    conn.upload_tree(
                        &local,
                        &p.dest,
                        &dest_name,
                        action,
                        p.keep_mtime,
                        &job.ctl,
                        &mut HashSet::new(),
                        0,
                        &mut out,
                    )
                    .await?;
                } else if conn
                    .upload_file_with(
                        &local,
                        &p.dest,
                        &name,
                        it.action,
                        p.keep_mtime,
                        &job.ctl,
                        p.risk,
                    )
                    .await?
                {
                    out.files += 1;
                } else {
                    out.skipped.push(format!("{name}: already exists"));
                }
            }
            Kind::Download => {
                let name = remote_path::file_name(&it.path).to_string();
                let m = conn
                    .sftp
                    .metadata(it.path.clone())
                    .await
                    .map_err(|e| XferError::Msg(crate::sftp::conn::describe(e)))?;
                let dir = Path::new(&p.dest);
                std::fs::create_dir_all(dir)
                    .map_err(|e| XferError::Msg(format!("{}: {e}", p.dest)))?;
                if m.file_type().is_dir() {
                    let mut action = it.action;
                    let mut dest_name = name.clone();
                    if dir.join(&name).symlink_metadata().is_ok() && action == Action::Rename {
                        dest_name = (1..)
                            .map(|n| crate::sftp::xfer::numbered(&name, n))
                            .find(|c| dir.join(c).symlink_metadata().is_err())
                            .expect("endless");
                        action = Action::Replace;
                    }
                    conn.download_tree(
                        &it.path,
                        dir,
                        &dest_name,
                        action,
                        p.keep_mtime,
                        &job.ctl,
                        0,
                        &mut out,
                    )
                    .await?;
                } else {
                    let exists = dir.join(&name).symlink_metadata().is_ok();
                    let (dest_name, overwrite) = match (exists, it.action) {
                        (false, _) => (name.clone(), false),
                        (true, Action::Skip) => {
                            out.skipped.push(format!("{name}: already exists"));
                            continue;
                        }
                        (true, Action::Replace) => (name.clone(), true),
                        (true, Action::Rename) => (
                            (1..)
                                .map(|n| crate::sftp::xfer::numbered(&name, n))
                                .find(|c| dir.join(c).symlink_metadata().is_err())
                                .expect("endless"),
                            false,
                        ),
                    };
                    conn.download_file(
                        &it.path,
                        dir,
                        &dest_name,
                        overwrite,
                        p.keep_mtime,
                        &job.ctl,
                    )
                    .await?;
                    out.files += 1;
                }
            }
        }
        *job.outcome.lock().expect("lock") = out.clone();
    }
    *job.outcome.lock().expect("lock") = out;
    Ok(())
}
