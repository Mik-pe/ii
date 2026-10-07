//! Single-level directory scans and latest-request-wins workers.
//! Navigation and previews use independent workers: a slow preview cannot block navigation.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;

use crate::filter::safe_label;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub label: String,
    pub folded: String,
    pub hidden: bool,
    pub symlink: bool,
}

impl Entry {
    pub fn new(path: PathBuf, symlink: bool) -> Self {
        let name = path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy();
        let hidden = name.starts_with('.');
        let label = safe_label(&name);
        let folded = label.to_lowercase();
        Self {
            path,
            label,
            folded,
            hidden,
            symlink,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub skipped: usize,
}

/// Scan exactly one directory. Regular files are not stat-ed or displayed.
pub fn scan(path: &Path) -> io::Result<Listing> {
    scan_cancellable(path, || false).unwrap_or_else(|| Ok(Listing::default()))
}

fn scan_cancellable(path: &Path, cancelled: impl Fn() -> bool) -> Option<io::Result<Listing>> {
    let directory = match fs::read_dir(path) {
        Ok(directory) => directory,
        Err(error) => return Some(Err(error)),
    };
    let mut listing = Listing::default();
    for item in directory {
        if cancelled() {
            return None;
        }
        let item = match item {
            Ok(item) => item,
            Err(_) => {
                listing.skipped += 1;
                continue;
            }
        };
        let kind = match item.file_type() {
            Ok(kind) => kind,
            Err(_) => {
                listing.skipped += 1;
                continue;
            }
        };
        if kind.is_dir() {
            listing.entries.push(Entry::new(item.path(), false));
        } else if kind.is_symlink() {
            // Only symlinks need metadata; never recursively follow them.
            match fs::metadata(item.path()) {
                Ok(metadata) if metadata.is_dir() => {
                    listing.entries.push(Entry::new(item.path(), true))
                }
                Ok(_) => {}
                Err(_) => listing.skipped += 1,
            }
        }
    }
    if cancelled() {
        return None;
    }
    listing
        .entries
        .sort_unstable_by(|a, b| a.folded.cmp(&b.folded).then_with(|| a.path.cmp(&b.path)));
    Some(Ok(listing))
}

#[derive(Debug)]
struct Request {
    id: u64,
    path: PathBuf,
}

#[derive(Debug)]
pub struct ScanResult {
    pub id: u64,
    pub path: PathBuf,
    pub result: io::Result<Arc<Listing>>,
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
    result: Mutex<Option<ScanResult>>,
    generation: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One pending request and one result slot, not an unbounded queue of obsolete work.
pub struct Scanner {
    shared: Arc<Shared>,
}

impl Scanner {
    pub fn new(name: &str) -> io::Result<Self> {
        let shared = Arc::new(Shared::default());
        let worker = Arc::clone(&shared);
        thread::Builder::new()
            .name(name.to_owned())
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
                    let is_obsolete = || worker.generation.load(Ordering::Acquire) != request.id;
                    if let Some(result) = scan_cancellable(&request.path, is_obsolete)
                        && !is_obsolete()
                    {
                        *lock(&worker.result) = Some(ScanResult {
                            id: request.id,
                            path: request.path,
                            result: result.map(Arc::new),
                        });
                    }
                }
            })?;
        Ok(Self { shared })
    }

    pub fn request(&self, path: PathBuf) -> u64 {
        let id = self.shared.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *lock(&self.shared.result) = None;
        lock(&self.shared.pending).request = Some(Request { id, path });
        self.shared.wake.notify_one();
        id
    }

    pub fn cancel(&self) {
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        lock(&self.shared.pending).request = None;
        *lock(&self.shared.result) = None;
    }

    pub fn poll(&self) -> Option<ScanResult> {
        lock(&self.shared.result).take()
    }
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.cancel();
        lock(&self.shared.pending).closed = true;
        self.shared.wake.notify_one();
        // Do not join a thread blocked inside an OS/network filesystem call.
        // Process exit must remain immediate; no writes are performed by workers.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn scans_only_directories_and_keeps_hidden_entries() {
        let root = tempfile::tempdir().unwrap();
        for name in ["zebra", "Alpha", ".hidden"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        fs::write(root.path().join("not-a-directory"), "").unwrap();
        let listing = scan(root.path()).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(names, [".hidden", "Alpha", "zebra"]);
        assert!(listing.entries[0].hidden);
        assert_eq!(listing.skipped, 0);
    }

    #[test]
    fn missing_directory_is_an_error_not_an_empty_listing() {
        let root = tempfile::tempdir().unwrap();
        assert!(scan(&root.path().join("missing")).is_err());
    }

    #[test]
    fn worker_delivers_latest_request() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let scanner = Scanner::new("ii-test").unwrap();
        scanner.request(root.path().to_owned());
        let expected = scanner.request(other.path().to_owned());
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = scanner.poll()
                && result.id == expected
            {
                assert_eq!(result.path, other.path());
                assert!(result.result.is_ok());
                break;
            }
            assert!(Instant::now() < deadline, "scan worker timed out");
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[cfg(unix)]
    #[test]
    fn follows_directory_symlinks_without_recursing_or_rewriting_paths() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        symlink(root.path().join("real"), root.path().join("alias")).unwrap();
        symlink(root.path().join("missing"), root.path().join("broken")).unwrap();
        let listing = scan(root.path()).unwrap();
        let alias = listing
            .entries
            .iter()
            .find(|entry| entry.label == "alias")
            .unwrap();
        assert!(alias.symlink);
        assert_eq!(alias.path, root.path().join("alias"));
        assert_eq!(listing.entries.len(), 2);
        assert_eq!(listing.skipped, 1);
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_paths_independently_of_display_labels() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(vec![b'a', 0xff]));
        let entry = Entry::new(path.clone(), false);
        assert_eq!(entry.path, path);
        assert!(entry.label.contains('\u{fffd}'));
    }

    // Linux's native test filesystem accepts arbitrary filename bytes. APFS
    // rejects malformed UTF-8 at creation; the pure invariant is tested above
    // on every Unix platform without pretending such a file can be created.
    #[cfg(target_os = "linux")]
    #[test]
    fn scans_non_utf8_directory_names() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(OsString::from_vec(vec![b'a', 0xff]));
        fs::create_dir(&path).unwrap();
        assert_eq!(scan(root.path()).unwrap().entries[0].path, path);
    }
}
