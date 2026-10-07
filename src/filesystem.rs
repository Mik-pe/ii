//! Single-level scans and latest-request-wins workers.
//! Entry classification never reads file contents or executes a program.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread;

use crate::filter::safe_label;

/// A visual hint inferred from the name, not a claim about the file's contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    Code,
    Config,
    Document,
    Media,
    Archive,
    Other,
}

impl FileKind {
    pub fn from_path(path: &Path) -> Self {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let name = name.to_ascii_lowercase();
        if matches!(
            name.as_str(),
            "dockerfile" | "makefile" | "justfile" | "gemfile" | "rakefile"
        ) {
            return Self::Code;
        }
        if matches!(
            name.as_str(),
            ".gitignore" | ".gitattributes" | ".editorconfig" | ".npmrc" | ".env"
        ) || name.starts_with(".env.")
        {
            return Self::Config;
        }
        if matches!(
            name.as_str(),
            "readme" | "license" | "licence" | "copying" | "changelog" | "authors"
        ) {
            return Self::Document;
        }
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        match extension.as_str() {
            "rs" | "py" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "go" | "c" | "h" | "cc"
            | "cpp" | "hpp" | "cs" | "java" | "kt" | "swift" | "rb" | "php" | "lua" | "sh"
            | "bash" | "zsh" | "fish" | "ps1" | "bat" | "cmd" | "html" | "css" | "scss" | "vue"
            | "svelte" | "sql" | "zig" | "ex" | "exs" => Self::Code,
            "toml" | "json" | "jsonc" | "yaml" | "yml" | "ini" | "cfg" | "conf" | "xml"
            | "lock" | "env" | "properties" => Self::Config,
            "md" | "mdx" | "txt" | "rst" | "adoc" | "pdf" | "doc" | "docx" | "odt" | "csv"
            | "tsv" | "xls" | "xlsx" | "ppt" | "pptx" => Self::Document,
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "svg" | "avif" | "ico" | "bmp" | "tiff"
            | "mp3" | "wav" | "flac" | "ogg" | "m4a" | "mp4" | "mkv" | "mov" | "webm" => {
                Self::Media
            }
            "zip" | "tar" | "gz" | "bz2" | "xz" | "zst" | "7z" | "rar" | "tgz" => Self::Archive,
            _ => Self::Other,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Config => "config",
            Self::Document => "document",
            Self::Media => "media",
            Self::Archive => "archive",
            Self::Other => "file",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Directory,
    File(FileKind),
    /// Includes broken links and links whose target cannot be inspected.
    UnresolvedLink,
    /// Sockets, FIFOs, devices, and other non-regular entries. Never opened.
    Special,
}

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub label: String,
    pub folded: String,
    pub hidden: bool,
    pub symlink: bool,
    pub kind: EntryKind,
}

impl Entry {
    /// Construct a directory entry. This also keeps model fixtures concise.
    pub fn new(path: PathBuf, symlink: bool) -> Self {
        Self::with_kind(path, symlink, EntryKind::Directory)
    }

    pub fn file(path: PathBuf, symlink: bool) -> Self {
        let kind = FileKind::from_path(&path);
        Self::with_kind(path, symlink, EntryKind::File(kind))
    }

    pub fn with_kind(path: PathBuf, symlink: bool, kind: EntryKind) -> Self {
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
            kind,
        }
    }

    pub fn is_dir(&self) -> bool {
        self.kind == EntryKind::Directory
    }

    pub fn type_label(&self) -> &'static str {
        match self.kind {
            EntryKind::Directory => "folder",
            EntryKind::File(kind) => kind.label(),
            EntryKind::UnresolvedLink => "unresolved link",
            EntryKind::Special => "special",
        }
    }

    pub fn suffix(&self) -> &'static str {
        match (self.kind, self.symlink) {
            (EntryKind::Directory, false) => "/",
            (EntryKind::Directory, true) => "/ ↗",
            (EntryKind::UnresolvedLink, _) => " ↗!",
            (_, true) => " ↗",
            _ => "",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Listing {
    pub entries: Vec<Entry>,
    pub skipped: usize,
}

/// Scan exactly one directory. Ordinary files need no extra metadata calls.
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
        let path = item.path();
        let entry = if kind.is_symlink() {
            // Inspect a link's target type only, retaining its original alias path.
            match fs::metadata(&path) {
                Ok(metadata) if metadata.is_dir() => Entry::new(path, true),
                Ok(metadata) if metadata.is_file() => Entry::file(path, true),
                Ok(_) => Entry::with_kind(path, true, EntryKind::Special),
                Err(_) => Entry::with_kind(path, true, EntryKind::UnresolvedLink),
            }
        } else if kind.is_dir() {
            Entry::new(path, false)
        } else if kind.is_file() {
            Entry::file(path, false)
        } else {
            Entry::with_kind(path, false, EntryKind::Special)
        };
        listing.entries.push(entry);
    }
    if cancelled() {
        return None;
    }
    listing.entries.sort_unstable_by(|a, b| {
        (!a.is_dir())
            .cmp(&!b.is_dir())
            .then_with(|| a.folded.cmp(&b.folded))
            .then_with(|| a.path.cmp(&b.path))
    });
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
        // Never join a worker blocked inside an OS/network filesystem call.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn scans_files_and_directories_with_directories_first() {
        let root = tempfile::tempdir().unwrap();
        for name in ["zebra", "Alpha", ".hidden"] {
            fs::create_dir(root.path().join(name)).unwrap();
        }
        for name in ["README.md", "a.rs", ".env"] {
            fs::write(root.path().join(name), "").unwrap();
        }
        let listing = scan(root.path()).unwrap();
        let names: Vec<_> = listing
            .entries
            .iter()
            .map(|entry| entry.label.as_str())
            .collect();
        assert_eq!(
            names,
            [".hidden", "Alpha", "zebra", ".env", "a.rs", "README.md"]
        );
        assert!(listing.entries[0].hidden);
        assert!(listing.entries[..3].iter().all(Entry::is_dir));
        assert_eq!(listing.entries[4].kind, EntryKind::File(FileKind::Code));
        assert_eq!(listing.skipped, 0);
    }

    #[test]
    fn classification_is_case_insensitive_and_does_not_require_a_file() {
        for (name, kind) in [
            ("MAIN.RS", FileKind::Code),
            ("Dockerfile", FileKind::Code),
            ("justfile", FileKind::Code),
            ("Cargo.toml", FileKind::Config),
            (".env.local", FileKind::Config),
            (".gitignore", FileKind::Config),
            ("README", FileKind::Document),
            ("notes.md", FileKind::Document),
            ("cover.WEBP", FileKind::Media),
            ("backup.tar.gz", FileKind::Archive),
            ("unknown", FileKind::Other),
            ("README.rs", FileKind::Code),
        ] {
            assert_eq!(FileKind::from_path(Path::new(name)), kind, "{name}");
        }
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
    fn keeps_directory_file_and_unresolved_symlinks_without_recursing() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("real")).unwrap();
        fs::write(root.path().join("file.txt"), "").unwrap();
        symlink(root.path().join("real"), root.path().join("alias")).unwrap();
        symlink(root.path().join("file.txt"), root.path().join("linked.txt")).unwrap();
        symlink(root.path().join("missing"), root.path().join("broken")).unwrap();
        let listing = scan(root.path()).unwrap();
        let find = |label: &str| {
            listing
                .entries
                .iter()
                .find(|entry| entry.label == label)
                .unwrap()
        };
        assert!(find("alias").is_dir());
        assert!(find("alias").symlink);
        assert_eq!(find("alias").path, root.path().join("alias"));
        assert_eq!(find("linked.txt").kind, EntryKind::File(FileKind::Document));
        assert!(find("linked.txt").symlink);
        assert_eq!(find("broken").kind, EntryKind::UnresolvedLink);
        assert_eq!(find("broken").suffix(), " ↗!");
        assert_eq!(listing.entries.len(), 5);
        assert_eq!(listing.skipped, 0);
    }

    #[cfg(unix)]
    #[test]
    fn sockets_are_classified_without_opening_them() {
        use std::os::unix::net::UnixListener;
        let root = tempfile::tempdir().unwrap();
        let _listener = UnixListener::bind(root.path().join("socket")).unwrap();
        let listing = scan(root.path()).unwrap();
        assert_eq!(listing.entries[0].kind, EntryKind::Special);
        assert!(!listing.entries[0].is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn preserves_non_utf8_paths_independently_of_display_labels() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(vec![b'a', 0xff]));
        for entry in [
            Entry::new(path.clone(), false),
            Entry::file(path.clone(), false),
        ] {
            assert_eq!(entry.path, path);
            assert!(entry.label.contains('\u{fffd}'));
        }
    }

    // APFS rejects malformed UTF-8; the pure path invariant above runs on all Unix.
    #[cfg(target_os = "linux")]
    #[test]
    fn scans_non_utf8_directory_and_file_names() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join(OsString::from_vec(vec![b'd', 0xff]));
        let file = root.path().join(OsString::from_vec(vec![b'f', 0xff]));
        fs::create_dir(&directory).unwrap();
        fs::write(&file, []).unwrap();
        let listing = scan(root.path()).unwrap();
        assert_eq!(listing.entries[0].path, directory);
        assert_eq!(listing.entries[1].path, file);
    }
}
