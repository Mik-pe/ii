//! Opt-in, bounded descendant-directory search. No index and no work until asked.
//! One lazy worker is independent of navigation/preview I/O. Results are directory
//! paths with relative display labels; files and symlink targets are never opened.

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use crate::filesystem::{Entry, Listing};
use crate::filter::{safe_label, score};

pub const DEBOUNCE: Duration = Duration::from_millis(100);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Query {
    pub root: PathBuf,
    pub text: String,
    pub hidden: bool,
    /// Explicit refresh or invalidation, even if the text has not changed.
    pub revision: u64,
}

#[derive(Clone, Copy)]
pub struct Limits {
    pub entries: usize,
    pub directories: usize,
    pub depth: usize,
    pub queued: usize,
    pub results: usize,
    pub time: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            entries: 100_000,
            directories: 10_000,
            depth: 32,
            queued: 4_096,
            results: 200,
            time: Duration::from_secs(2),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Progress {
    pub listing: Arc<Listing>,
    pub visited: usize,
    pub skipped: usize,
    pub limited: bool,
    pub done: bool,
    pub error: Option<String>,
}

/// Fixed exclusions for traversal only. The excluded directory itself may match.
/// This intentionally does not pretend to implement arbitrary .gitignore rules.
fn pruned(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "node_modules"
            | "target"
            | ".venv"
            | "venv"
            | "__pycache__"
            | ".cache"
    )
}

fn relative_label(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .components()
        .map(|part| safe_label(&part.as_os_str().to_string_lossy()))
        .collect::<Vec<_>>()
        .join("/")
}

// Prefer basename matches to incidental matches in ancestor names; within a
// group prefer compact subsequences, shallow results and deterministic paths.
type Rank = (u8, usize, usize, String, PathBuf);

fn ranking(entry: &Entry, query: &str, depth: usize) -> Option<Rank> {
    let basename = entry.folded.rsplit('/').next().unwrap_or(&entry.folded);
    if !query.contains('/')
        && let Some(cost) = score(basename, query)
    {
        return Some((0, cost, depth, entry.folded.clone(), entry.path.clone()));
    }
    score(&entry.folded, query)
        .map(|cost| (1, cost, depth, entry.folded.clone(), entry.path.clone()))
}

/// Blocking search used by the worker and benchmarks, not by the UI thread.
/// Cancellation is checked between OS calls; an OS call itself may still block.
pub fn search(
    query: &Query,
    limits: Limits,
    cancelled: impl Fn() -> bool,
    mut publish: impl FnMut(Progress),
) {
    if cancelled() {
        return;
    }
    let started = Instant::now();
    let mut emitted = started;
    let text = query.text.to_lowercase();
    let mut progress = Progress::default();
    if text.is_empty() {
        progress.done = true;
        publish(progress);
        return;
    }
    let mut best = BTreeMap::<Rank, Entry>::new();
    let mut pending = VecDeque::from([(query.root.clone(), 0usize)]);
    let mut directories = 0usize;
    'walk: while let Some((path, depth)) = pending.pop_front() {
        if cancelled() {
            return;
        }
        if directories >= limits.directories || started.elapsed() >= limits.time {
            progress.limited = true;
            break;
        }
        directories += 1;
        let directory = match fs::read_dir(&path) {
            Ok(directory) => directory,
            Err(error) => {
                progress.skipped += 1;
                if path == query.root {
                    progress.error = Some(safe_label(&error.to_string()));
                }
                continue;
            }
        };
        for item in directory {
            if cancelled() {
                return;
            }
            if progress.visited >= limits.entries || started.elapsed() >= limits.time {
                progress.limited = true;
                break 'walk;
            }
            progress.visited += 1;
            // Also publish progress through huge file-heavy directories, not only
            // when a directory matches. The mailbox retains just one snapshot.
            if emitted.elapsed() >= Duration::from_millis(50) {
                progress.listing = Arc::new(Listing {
                    entries: best.values().cloned().collect(),
                    skipped: 0,
                });
                publish(progress.clone());
                emitted = Instant::now();
            }
            let item = match item {
                Ok(item) => item,
                Err(_) => {
                    progress.skipped += 1;
                    continue;
                }
            };
            let kind = match item.file_type() {
                Ok(kind) => kind,
                Err(_) => {
                    progress.skipped += 1;
                    continue;
                }
            };
            // No following symlinks: no cycles, target stats or escapes through
            // symlinks. Ordinary browsing still supports directory symlinks.
            if !kind.is_dir() || kind.is_symlink() {
                continue;
            }
            let name = item.file_name();
            let name = name.to_string_lossy();
            if !query.hidden && name.starts_with('.') {
                continue;
            }
            let child = item.path();
            let mut entry = Entry::new(child.clone(), false);
            entry.label = relative_label(&child, &query.root);
            entry.folded = entry.label.to_lowercase();
            if let Some(rank) = ranking(&entry, &text, depth + 1) {
                best.insert(rank, entry);
                if best.len() > limits.results {
                    best.pop_last();
                    progress.limited = true;
                }
            }
            if pruned(&name) {
                continue;
            }
            if depth + 1 >= limits.depth || pending.len() >= limits.queued {
                progress.limited = true;
                continue;
            }
            pending.push_back((child, depth + 1));
        }
    }
    if cancelled() {
        return;
    }
    progress.listing = Arc::new(Listing {
        entries: best.into_values().collect(),
        skipped: 0,
    });
    progress.done = true;
    publish(progress);
}

struct Request {
    id: u64,
    query: Query,
}
struct Response {
    id: u64,
    progress: Progress,
}
#[derive(Default)]
struct Pending {
    request: Option<Request>,
    closed: bool,
}
#[derive(Default)]
struct Shared {
    pending: Mutex<Pending>,
    wake: Condvar,
    result: Mutex<Option<Response>>,
    generation: AtomicU64,
}
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

struct Worker {
    shared: Arc<Shared>,
}
impl Worker {
    fn new() -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name("ii-deep-search".into())
            .spawn(move || {
                loop {
                    let request = {
                        let mut pending = lock(&worker.pending);
                        while pending.request.is_none() && !pending.closed {
                            pending = worker
                                .wake
                                .wait(pending)
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                        }
                        if pending.closed {
                            return;
                        }
                        pending.request.take()
                    };
                    let Some(request) = request else { continue };
                    let obsolete = || worker.generation.load(Ordering::Acquire) != request.id;
                    search(&request.query, Limits::default(), obsolete, |progress| {
                        let mut result = lock(&worker.result);
                        if !obsolete() {
                            *result = Some(Response {
                                id: request.id,
                                progress,
                            });
                        }
                    });
                }
            })?;
        Ok(Self { shared })
    }
    fn request(&self, query: Query) -> u64 {
        let id = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *lock(&self.shared.result) = None;
        lock(&self.shared.pending).request = Some(Request { id, query });
        self.shared.wake.notify_one();
        id
    }
    fn cancel(&self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        lock(&self.shared.pending).request = None;
        *lock(&self.shared.result) = None;
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.cancel();
        lock(&self.shared.pending).closed = true;
        self.shared.wake.notify_one();
        // Do not join a thread that could be blocked in a filesystem system call.
    }
}

/// No worker, thread, recursive I/O or index allocation for ordinary navigation.
/// A changed query cancels old work immediately; the next scan is debounced.
#[derive(Default)]
pub struct Controller {
    key: Option<Query>,
    due: Option<Instant>,
    worker: Option<Worker>,
    active: Option<u64>,
}
impl Controller {
    pub fn update(&mut self, key: Option<Query>, now: Instant) -> io::Result<Option<Progress>> {
        let key = key.filter(|query| !query.text.is_empty());
        if self.key != key {
            if let Some(worker) = &self.worker {
                worker.cancel();
            }
            self.active = None;
            self.due = key.as_ref().map(|_| now + DEBOUNCE);
            self.key = key;
        }
        if self.key.is_none() {
            return Ok(None);
        }
        if self.due.is_some_and(|due| now >= due) {
            self.due = None;
            if self.worker.is_none() {
                self.worker = Some(Worker::new()?);
            }
            if let (Some(worker), Some(key)) = (&self.worker, &self.key) {
                self.active = Some(worker.request(key.clone()));
            }
        }
        if let Some(worker) = &self.worker
            && let Some(response) = lock(&worker.shared.result).take()
            && Some(response.id) == self.active
        {
            if response.progress.done {
                self.active = None;
            }
            return Ok(Some(response.progress));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn query(root: &Path, text: &str) -> Query {
        Query {
            root: root.to_owned(),
            text: text.into(),
            hidden: false,
            revision: 0,
        }
    }
    fn run(query: &Query, limits: Limits) -> Progress {
        let mut result = None;
        search(query, limits, || false, |progress| result = Some(progress));
        result.unwrap()
    }

    #[test]
    fn finds_deep_relative_paths_and_prefers_basename_matches() {
        let root = tempfile::tempdir().unwrap();
        for name in [
            "server/services/api",
            "client/api",
            "api/other",
            "target/api",
            "node_modules/api",
            ".hidden/api",
        ] {
            fs::create_dir_all(root.path().join(name)).unwrap();
        }
        fs::write(root.path().join("api.txt"), "not a directory").unwrap();
        let result = run(&query(root.path(), "api"), Limits::default());
        let names: Vec<_> = result
            .listing
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(
            names,
            ["api", "client/api", "server/services/api", "api/other"]
        );
        assert!(!result.limited);
        assert!(result.done);
        assert!(
            result
                .listing
                .entries
                .iter()
                .all(|entry| entry.is_dir() && entry.path.is_absolute())
        );
        let result = run(&query(root.path(), "server/api"), Limits::default());
        assert_eq!(result.listing.entries[0].label, "server/services/api");
    }

    #[test]
    fn hidden_toggle_is_explicit_and_pruned_roots_can_still_be_selected() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join(".hidden/deep")).unwrap();
        fs::create_dir_all(root.path().join("target/deep")).unwrap();
        let mut request = query(root.path(), "deep");
        assert!(run(&request, Limits::default()).listing.entries.is_empty());
        request.hidden = true;
        assert_eq!(
            run(&request, Limits::default()).listing.entries[0].label,
            ".hidden/deep"
        );
        let result = run(&query(root.path(), "target"), Limits::default());
        assert_eq!(result.listing.entries[0].path, root.path().join("target"));
    }

    #[test]
    fn results_entries_depth_and_queue_have_hard_bounds() {
        let root = tempfile::tempdir().unwrap();
        for n in 0..20 {
            fs::create_dir_all(root.path().join(format!("folder-{n:02}/deep"))).unwrap();
        }
        let request = query(root.path(), "folder");
        let result = run(
            &request,
            Limits {
                results: 3,
                ..Limits::default()
            },
        );
        assert_eq!(result.listing.entries.len(), 3);
        assert!(result.limited);
        let result = run(
            &request,
            Limits {
                entries: 5,
                ..Limits::default()
            },
        );
        assert_eq!(result.visited, 5);
        assert!(result.limited);
        let result = run(
            &query(root.path(), "deep"),
            Limits {
                depth: 1,
                ..Limits::default()
            },
        );
        assert!(result.listing.entries.is_empty());
        assert!(result.limited);
        let result = run(
            &request,
            Limits {
                queued: 0,
                ..Limits::default()
            },
        );
        assert!(result.limited);
        let result = run(
            &request,
            Limits {
                time: Duration::ZERO,
                ..Limits::default()
            },
        );
        assert_eq!(result.visited, 0);
        assert!(result.limited);
    }

    #[test]
    fn empty_cancelled_and_inactive_search_do_not_touch_the_filesystem() {
        let missing = Path::new("/this-path-does-not-exist-ii-test");
        let empty = run(&query(missing, ""), Limits::default());
        assert!(empty.error.is_none());
        assert_eq!(empty.visited, 0);
        let emitted = Cell::new(false);
        search(
            &query(missing, "anything"),
            Limits::default(),
            || true,
            |_| emitted.set(true),
        );
        assert!(!emitted.get());
        let mut controller = Controller::default();
        let now = Instant::now();
        for _ in 0..100 {
            controller.update(None, now).unwrap();
            controller.update(Some(query(missing, "")), now).unwrap();
        }
        assert!(controller.worker.is_none());
        controller
            .update(Some(query(missing, "anything")), now)
            .unwrap();
        assert!(controller.worker.is_none(), "must debounce before spawning");
        controller.update(None, now + DEBOUNCE).unwrap();
        assert!(
            controller.worker.is_none(),
            "cancelled debounce must never launch"
        );
    }

    #[test]
    fn missing_root_is_not_reported_as_a_successful_empty_search() {
        let root = tempfile::tempdir().unwrap();
        let result = run(
            &query(&root.path().join("missing"), "foo"),
            Limits::default(),
        );
        assert!(result.done);
        assert!(result.error.is_some());
        assert_eq!(result.skipped, 1);
    }

    #[test]
    fn query_changes_and_cancellation_do_not_deliver_stale_results() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("alpha")).unwrap();
        fs::create_dir(root.path().join("beta")).unwrap();
        let now = Instant::now();
        let mut controller = Controller::default();
        controller
            .update(Some(query(root.path(), "alpha")), now)
            .unwrap();
        controller
            .update(Some(query(root.path(), "alpha")), now + DEBOUNCE)
            .unwrap();
        let next = query(root.path(), "beta");
        assert!(
            controller
                .update(Some(next.clone()), now + DEBOUNCE)
                .unwrap()
                .is_none()
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = controller
                .update(Some(next.clone()), now + DEBOUNCE * 2)
                .unwrap()
            {
                assert!(
                    result
                        .listing
                        .entries
                        .iter()
                        .all(|entry| entry.label == "beta")
                );
                if result.done {
                    assert_eq!(result.listing.entries.len(), 1);
                    break;
                }
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(1));
        }
        controller.update(None, Instant::now()).unwrap();
        assert!(controller.active.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn does_not_follow_symlink_cycles_or_leave_root_through_links() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::create_dir(outside.path().join("secret")).unwrap();
        symlink(root.path(), root.path().join("real/loop")).unwrap();
        symlink(outside.path(), root.path().join("escape")).unwrap();
        let result = run(&query(root.path(), "secret"), Limits::default());
        assert!(result.listing.entries.is_empty());
        assert!(result.done && !result.limited);
    }
}
