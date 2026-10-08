//! Pure navigation state. No terminal I/O and no synchronous filesystem calls.

use crate::deep::{Progress, Query};
use crate::filesystem::{Entry, EntryKind, Listing};
use crate::filter;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

const CACHE_DIRECTORIES: usize = 32;
const CACHE_ENTRIES: usize = 100_000;
const MEMORY_DIRECTORIES: usize = 128;
const QUERY_CHARACTERS: usize = 256;

#[derive(Default)]
struct Cache {
    items: VecDeque<(PathBuf, Arc<Listing>)>,
    entries: usize,
}

impl Cache {
    fn get(&mut self, path: &Path) -> Option<Arc<Listing>> {
        let index = self
            .items
            .iter()
            .position(|(candidate, _)| candidate == path)?;
        let item = self.items.remove(index)?;
        let listing = Arc::clone(&item.1);
        self.items.push_back(item);
        Some(listing)
    }

    fn insert(&mut self, path: PathBuf, listing: Arc<Listing>) {
        if let Some(index) = self
            .items
            .iter()
            .position(|(candidate, _)| candidate == &path)
            && let Some((_, old)) = self.items.remove(index)
        {
            self.entries -= old.entries.len();
        }
        if listing.entries.len() > CACHE_ENTRIES {
            return;
        }
        while self.items.len() >= CACHE_DIRECTORIES
            || self.entries + listing.entries.len() > CACHE_ENTRIES
        {
            if let Some((_, old)) = self.items.pop_front() {
                self.entries -= old.entries.len();
            } else {
                break;
            }
        }
        self.entries += listing.entries.len();
        self.items.push_back((path, listing));
    }
}

#[derive(Clone)]
struct Location {
    path: PathBuf,
    listing: Arc<Listing>,
    selected: Option<PathBuf>,
    query: String,
    offset: usize,
}

/// Recomputed only when visibility changes, never on every arrow key or render.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Counts {
    pub folders: usize,
    pub files: usize,
    pub other: usize,
}

/// Transient search view; results never enter the local listing cache.
pub struct DeepState {
    local: Location,
    pub progress: Progress,
}

pub struct Model {
    pub cwd: PathBuf,
    pub listing: Option<Arc<Listing>>,
    pub visible: Vec<usize>,
    pub counts: Counts,
    pub selected: usize,
    pub offset: usize,
    pub query: String,
    pub show_hidden: bool,
    pub dirs_only: bool,
    pub loading: bool,
    pub message: Option<String>,
    pub help: bool,
    pub help_offset: usize,
    pub deep: Option<DeepState>,
    deep_revision: u64,
    pub preview_path: Option<PathBuf>,
    pub preview: Option<Arc<Listing>>,
    pub preview_error: Option<String>,
    pub preview_loading: bool,
    cache: Cache,
    memory: VecDeque<(PathBuf, PathBuf)>,
    preferred: Option<PathBuf>,
    fallback: Option<Location>,
}

impl Model {
    pub fn new(cwd: PathBuf, show_hidden: bool) -> Self {
        Self {
            cwd,
            show_hidden,
            dirs_only: false,
            listing: None,
            visible: Vec::new(),
            counts: Counts::default(),
            selected: 0,
            offset: 0,
            query: String::new(),
            loading: true,
            message: None,
            help: false,
            help_offset: 0,
            deep: None,
            deep_revision: 0,
            preview_path: None,
            preview: None,
            preview_error: None,
            preview_loading: false,
            cache: Cache::default(),
            memory: VecDeque::new(),
            preferred: None,
            fallback: None,
        }
    }

    pub fn start_deep(&mut self) {
        if self.deep.is_some() {
            return;
        }
        if self.loading {
            self.message = Some("Wait for the current directory before searching below it.".into());
            return;
        }
        let Some(listing) = &self.listing else { return };
        self.deep = Some(DeepState {
            local: Location {
                path: self.cwd.clone(),
                listing: Arc::clone(listing),
                selected: self.selected_path(),
                query: self.query.clone(),
                offset: self.offset,
            },
            progress: Progress::default(),
        });
        self.refresh_deep();
    }

    pub fn leave_deep(&mut self) {
        let Some(deep) = self.deep.take() else { return };
        self.cwd = deep.local.path;
        self.listing = Some(deep.local.listing);
        self.query = deep.local.query;
        self.message = None;
        self.rebuild(deep.local.selected);
        self.offset = deep.local.offset;
    }

    pub fn refresh_deep(&mut self) {
        if let Some(deep) = &mut self.deep {
            self.deep_revision = self.deep_revision.wrapping_add(1);
            deep.progress = Progress {
                done: self.query.is_empty(),
                ..Progress::default()
            };
            self.listing = Some(Arc::new(Listing::default()));
            self.visible.clear();
            self.counts = Counts::default();
            self.selected = 0;
            self.offset = 0;
            self.message = None;
        }
    }

    pub fn deep_query(&self) -> Option<Query> {
        self.deep.as_ref().map(|_| Query {
            root: self.cwd.clone(),
            text: self.query.clone(),
            hidden: self.show_hidden,
            revision: self.deep_revision,
        })
    }

    pub fn apply_deep(&mut self, progress: Progress) {
        let selected = self.selected_path();
        let Some(deep) = &mut self.deep else { return };
        self.listing = Some(Arc::clone(&progress.listing));
        self.visible = (0..progress.listing.entries.len()).collect();
        self.counts = Counts {
            folders: self.visible.len(),
            files: 0,
            other: 0,
        };
        self.selected = selected
            .and_then(|path| {
                progress
                    .listing
                    .entries
                    .iter()
                    .position(|entry| entry.path == path)
            })
            .unwrap_or(0);
        // Streamed batches keep selection by original path, not shifting rank.
        deep.progress = progress;
    }

    pub fn selected_entry(&self) -> Option<&Entry> {
        let index = *self.visible.get(self.selected)?;
        self.listing.as_ref()?.entries.get(index)
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_entry().map(|entry| entry.path.clone())
    }

    /// Used by preview scheduling: files must never be passed to a directory scan.
    pub fn selected_directory(&self) -> Option<PathBuf> {
        self.selected_entry()
            .filter(|entry| entry.is_dir())
            .map(|entry| entry.path.clone())
    }

    /// Tab, local Right and deep-search Enter use this directory-only navigation guard.
    pub fn directory_target(&mut self) -> Option<PathBuf> {
        if let Some(path) = self.selected_directory() {
            return Some(path);
        }
        self.message = self.selected_entry().map(|entry| {
            match entry.kind {
                EntryKind::UnresolvedLink => {
                    "Link target unavailable. Enter changes to the current folder."
                }
                EntryKind::Special => {
                    "Special entry; not opened. Enter changes to the current folder."
                }
                _ => "File selected. Enter changes to this folder; Ctrl-F hides files.",
            }
            .to_owned()
        });
        None
    }

    /// Visibility for the next-directory pane (its contents have their own names).
    pub fn preview_visible(&self, entry: &Entry) -> bool {
        (self.show_hidden || !entry.hidden) && (!self.dirs_only || entry.is_dir())
    }

    pub fn cached(&mut self, path: &Path) -> Option<Arc<Listing>> {
        self.cache.get(path)
    }

    pub fn cache_listing(&mut self, path: PathBuf, listing: Arc<Listing>) {
        self.cache.insert(path, listing);
    }

    fn remember(&mut self) {
        if let Some(selected) = self.selected_path() {
            self.memory.retain(|(path, _)| path != &self.cwd);
            self.memory.push_back((self.cwd.clone(), selected));
            if self.memory.len() > MEMORY_DIRECTORIES {
                self.memory.pop_front();
            }
        }
    }

    /// Show cached contents immediately; a worker always revalidates the directory.
    pub fn begin_navigation(&mut self, path: PathBuf, preferred: Option<PathBuf>) {
        self.leave_deep();
        let refresh = path == self.cwd;
        let offset = if refresh { self.offset } else { 0 };
        self.remember();
        if !self.loading
            && let Some(listing) = &self.listing
        {
            self.fallback = Some(Location {
                path: self.cwd.clone(),
                listing: Arc::clone(listing),
                selected: self.selected_path(),
                query: self.query.clone(),
                offset: self.offset,
            });
        }
        self.preferred = preferred.or_else(|| {
            self.memory
                .iter()
                .find(|(candidate, _)| candidate == &path)
                .map(|(_, selected)| selected.clone())
        });
        self.cwd = path;
        self.listing = if refresh {
            self.listing.clone()
        } else {
            self.cache.get(&self.cwd)
        };
        self.loading = true;
        self.message = None;
        if !refresh {
            self.query.clear();
        }
        self.rebuild(self.preferred.clone());
        self.offset = offset;
    }

    pub fn complete_navigation(&mut self, listing: Arc<Listing>) {
        let offset = self.offset;
        let preferred = self.selected_path().or_else(|| self.preferred.take());
        self.cache.insert(self.cwd.clone(), Arc::clone(&listing));
        self.listing = Some(listing);
        self.loading = false;
        self.fallback = None;
        self.rebuild(preferred);
        self.offset = offset;
    }

    pub fn fail_navigation(&mut self, error: String) {
        if let Some(previous) = self.fallback.take() {
            self.cwd = previous.path;
            self.listing = Some(previous.listing);
            self.query = previous.query;
            self.rebuild(previous.selected);
            self.offset = previous.offset;
        } else {
            self.listing = None;
            self.visible.clear();
            self.counts = Counts::default();
            self.selected = 0;
        }
        self.loading = false;
        self.message = Some(filter::safe_label(&error));
    }

    /// Parent traversal explicitly selects the directory we just came out of.
    pub fn parent(&self) -> Option<(PathBuf, PathBuf)> {
        self.cwd
            .parent()
            .filter(|parent| *parent != self.cwd)
            .map(|parent| (parent.to_owned(), self.cwd.clone()))
    }

    pub fn move_selection(&mut self, amount: isize) {
        if self.visible.is_empty() {
            self.selected = 0;
            return;
        }
        self.selected = self
            .selected
            .saturating_add_signed(amount)
            .min(self.visible.len() - 1);
        self.message = None;
    }

    pub fn toggle_hidden(&mut self) {
        let selected = self.selected_path();
        self.show_hidden = !self.show_hidden;
        self.rebuild(selected);
        self.message = None;
    }

    pub fn toggle_files(&mut self) {
        if self.deep.is_some() {
            self.message =
                Some("Deep search finds folders. Esc returns to files and folders.".into());
            return;
        }
        let selected = self.selected_path();
        self.dirs_only = !self.dirs_only;
        self.rebuild(selected);
        self.message = None;
    }

    pub fn push_query(&mut self, text: &str) {
        let remaining = QUERY_CHARACTERS.saturating_sub(self.query.chars().count());
        self.query
            .extend(text.chars().filter(|ch| !ch.is_control()).take(remaining));
        self.rebuild(None);
        self.message = None;
    }

    pub fn pop_query(&mut self) {
        if let Some((index, _)) = self.query.grapheme_indices(true).next_back() {
            self.query.truncate(index);
        }
        self.rebuild(None);
    }

    pub fn clear_query(&mut self) {
        let selected = self.selected_path();
        self.query.clear();
        self.rebuild(selected);
    }

    fn rebuild(&mut self, preferred: Option<PathBuf>) {
        if self.deep.is_some() {
            self.refresh_deep();
            return;
        }
        self.visible.clear();
        self.counts = Counts::default();
        if let Some(listing) = &self.listing {
            let query = self.query.to_lowercase();
            let reveal_hidden = self.show_hidden || query.starts_with('.');
            let allowed = |entry: &Entry| {
                (reveal_hidden || !entry.hidden) && (!self.dirs_only || entry.is_dir())
            };
            if query.is_empty() {
                self.visible.extend(
                    listing
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| allowed(entry))
                        .map(|(index, _)| index),
                );
            } else {
                let mut ranked: Vec<_> = listing
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| allowed(entry))
                    .filter_map(|(index, entry)| {
                        filter::score(&entry.folded, &query)
                            .map(|score| (!entry.is_dir(), score, index))
                    })
                    .collect();
                ranked.sort_unstable();
                self.visible
                    .extend(ranked.into_iter().map(|(_, _, index)| index));
            }
            for &index in &self.visible {
                match listing.entries[index].kind {
                    EntryKind::Directory => self.counts.folders += 1,
                    EntryKind::File(_) => self.counts.files += 1,
                    _ => self.counts.other += 1,
                }
            }
            self.selected = preferred
                .and_then(|path| {
                    self.visible
                        .iter()
                        .position(|&index| listing.entries[index].path == path)
                })
                .unwrap_or(0);
        } else {
            self.selected = 0;
        }
        self.offset = 0;
    }

    /// The renderer only builds visible rows, even in directories with 100k entries.
    pub fn viewport(&mut self, height: usize) -> std::ops::Range<usize> {
        if height == 0 {
            return 0..0;
        }
        self.offset = self.offset.min(self.visible.len().saturating_sub(height));
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + height {
            self.offset = self.selected + 1 - height;
        }
        self.offset..(self.offset + height).min(self.visible.len())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listing(root: &Path, names: &[&str]) -> Arc<Listing> {
        Arc::new(Listing {
            entries: names
                .iter()
                .map(|name| Entry::new(root.join(name), false))
                .collect(),
            skipped: 0,
        })
    }

    fn model() -> Model {
        let root = PathBuf::from("/code");
        let mut model = Model::new(root.clone(), false);
        model.complete_navigation(listing(&root, &[".git", "app", "notes", "tools"]));
        model
    }

    fn mixed_model() -> Model {
        let mut model = Model::new(PathBuf::from("/code"), false);
        model.complete_navigation(Arc::new(Listing {
            entries: vec![
                Entry::new(PathBuf::from("/code/src"), false),
                Entry::file(PathBuf::from("/code/.env"), false),
                Entry::file(PathBuf::from("/code/main.rs"), false),
                Entry::file(PathBuf::from("/code/README.md"), false),
            ],
            skipped: 0,
        }));
        model
    }

    #[test]
    fn files_are_visible_filterable_and_never_navigation_targets() {
        let mut model = mixed_model();
        assert_eq!(
            model.counts,
            Counts {
                folders: 1,
                files: 2,
                other: 0
            }
        );
        model.push_query("mnrs");
        assert_eq!(model.selected_entry().unwrap().label, "main.rs");
        assert!(model.selected_directory().is_none());
        assert!(model.directory_target().is_none());
        assert!(model.message.as_deref().unwrap().contains("File selected"));
        assert_eq!(model.cwd, Path::new("/code"));
        assert_eq!(model.counts.files, 1);
    }

    #[test]
    fn toggling_files_preserves_directory_selection_and_the_filter() {
        let mut model = mixed_model();
        let selected = model.selected_path();
        model.toggle_files();
        assert_eq!(model.selected_path(), selected);
        assert_eq!(
            model.counts,
            Counts {
                folders: 1,
                files: 0,
                other: 0
            }
        );
        model.push_query("main");
        assert!(model.visible.is_empty());
        model.toggle_files();
        assert_eq!(model.query, "main");
        assert_eq!(model.selected_entry().unwrap().label, "main.rs");
        model.clear_query();
        model.toggle_files();
        assert!(model.selected_entry().unwrap().is_dir());
        assert_eq!(model.selected, 0);
    }

    #[test]
    fn hidden_files_follow_visibility_without_extra_scans() {
        let mut model = mixed_model();
        model.toggle_hidden();
        assert_eq!(model.counts.files, 3);
        model.toggle_hidden();
        model.push_query(".e");
        assert_eq!(model.selected_entry().unwrap().label, ".env");
        model.toggle_files();
        assert!(model.visible.is_empty());
    }

    #[test]
    fn matching_directories_still_precede_exact_file_matches() {
        let mut model = mixed_model();
        model.listing = Some(Arc::new(Listing {
            entries: vec![
                Entry::new(PathBuf::from("/code/rust-source"), false),
                Entry::file(PathBuf::from("/code/rs"), false),
            ],
            skipped: 0,
        }));
        model.push_query("rs");
        assert!(model.selected_entry().unwrap().is_dir());
        assert_eq!(model.visible.len(), 2);
    }

    #[test]
    fn file_only_listing_is_not_an_empty_directory() {
        let mut model = Model::new(PathBuf::from("/code"), false);
        model.complete_navigation(Arc::new(Listing {
            entries: vec![Entry::file(PathBuf::from("/code/a.txt"), false)],
            skipped: 0,
        }));
        assert_eq!(model.counts.files, 1);
        assert_eq!(model.visible.len(), 1);
        model.toggle_files();
        assert!(model.visible.is_empty());
        model.move_selection(1);
        assert_eq!(model.viewport(5), 0..0);
    }

    #[test]
    fn enter_parent_restores_selection() {
        let mut model = model();
        model.move_selection(2);
        let child = model.selected_path().unwrap();
        model.begin_navigation(child.clone(), None);
        model.complete_navigation(listing(&child, &["a", "b"]));
        let (parent, preferred) = model.parent().unwrap();
        model.begin_navigation(parent.clone(), Some(preferred));
        model.complete_navigation(listing(&parent, &[".git", "app", "notes", "tools"]));
        assert_eq!(model.selected_path().unwrap(), child);
    }

    #[test]
    fn remembers_selection_when_reentering() {
        let mut model = model();
        let child = model.selected_path().unwrap();
        model.begin_navigation(child.clone(), None);
        model.complete_navigation(listing(&child, &["a", "b", "c"]));
        model.move_selection(1);
        let (parent, preferred) = model.parent().unwrap();
        model.begin_navigation(parent.clone(), Some(preferred));
        model.complete_navigation(listing(&parent, &["app"]));
        model.begin_navigation(child.clone(), None);
        model.complete_navigation(listing(&child, &["a", "b", "c"]));
        assert_eq!(model.selected_path(), Some(child.join("b")));
    }

    #[test]
    fn failed_navigation_restores_location_and_explains_error() {
        let mut model = model();
        model.begin_navigation(PathBuf::from("/missing"), None);
        model.fail_navigation("Permission denied".into());
        assert_eq!(model.cwd, Path::new("/code"));
        assert!(!model.loading);
        assert_eq!(model.message.as_deref(), Some("Permission denied"));
        assert!(model.selected_entry().is_some());
    }

    #[test]
    fn refresh_preserves_filter_selection_and_visible_page() {
        let root = PathBuf::from("/code");
        let names: Vec<_> = (0..100).map(|index| format!("app-{index:03}")).collect();
        let names: Vec<_> = names.iter().map(String::as_str).collect();
        let original = listing(&root, &names);
        let mut model = Model::new(root.clone(), false);
        model.complete_navigation(Arc::clone(&original));
        model.push_query("ap");
        model.move_selection(37);
        let selected = model.selected_path();
        let page = model.viewport(5);
        model.begin_navigation(root.clone(), selected.clone());
        assert!(model.loading);
        assert_eq!(model.query, "ap");
        assert_eq!(model.viewport(5), page);
        assert_eq!(model.selected_path(), selected);

        let mut updated = original.as_ref().clone();
        updated
            .entries
            .push(Entry::new(root.join("app-new"), false));
        model.complete_navigation(Arc::new(updated));
        assert_eq!(model.query, "ap");
        assert_eq!(model.selected_path(), selected);
        assert_eq!(model.viewport(5), page);
        assert_eq!(model.counts.folders, 101);
    }

    #[test]
    fn failed_navigation_restores_filtered_view_and_scroll_after_chained_requests() {
        let root = PathBuf::from("/code");
        let mut model = Model::new(root.clone(), false);
        model.complete_navigation(Arc::new(Listing {
            entries: (0..40)
                .map(|index| Entry::new(root.join(format!("app-{index:03}")), false))
                .collect(),
            skipped: 0,
        }));
        model.push_query("ap");
        model.move_selection(25);
        let selected = model.selected_path();
        let page = model.viewport(5);
        model.begin_navigation(root.join("missing"), None);
        model.begin_navigation(root.join("also-missing"), None);
        model.fail_navigation("directory disappeared".into());
        assert_eq!(model.cwd, root);
        assert_eq!(model.query, "ap");
        assert_eq!(model.selected_path(), selected);
        assert_eq!(model.viewport(5), page);
        assert!(!model.loading);
    }

    #[test]
    fn refresh_revalidates_a_removed_selection_without_losing_the_filter() {
        let mut model = model();
        model.push_query("tl");
        let root = model.cwd.clone();
        model.begin_navigation(root.clone(), model.selected_path());
        model.complete_navigation(listing(&root, &["toolbox"]));
        assert_eq!(model.query, "tl");
        assert_eq!(model.selected_path(), Some(root.join("toolbox")));
    }

    #[test]
    fn hidden_filter_and_unicode_backspace() {
        let mut model = model();
        assert_eq!(model.visible.len(), 3);
        model.toggle_hidden();
        assert_eq!(model.visible.len(), 4);
        model.push_query("tl");
        assert_eq!(model.selected_entry().unwrap().label, "tools");
        model.clear_query();
        model.push_query("🧑‍💻");
        model.pop_query();
        assert!(model.query.is_empty());
        model.toggle_hidden();
        model.push_query(".g");
        assert_eq!(model.selected_entry().unwrap().label, ".git");
    }

    #[test]
    fn selection_clamps_and_empty_results_are_safe() {
        let mut model = model();
        model.move_selection(isize::MAX);
        assert_eq!(model.selected, 2);
        assert_eq!(model.viewport(1), 2..3);
        model.move_selection(isize::MIN);
        assert_eq!(model.selected, 0);
        model.push_query("no-match");
        model.move_selection(1);
        assert!(model.selected_entry().is_none());
        assert_eq!(model.viewport(10), 0..0);
    }

    #[test]
    fn query_length_is_bounded() {
        let mut model = model();
        model.push_query(&"x".repeat(10_000));
        assert_eq!(model.query.chars().count(), QUERY_CHARACTERS);
    }

    #[test]
    fn cache_is_bounded() {
        let mut cache = Cache::default();
        for index in 0..100 {
            cache.insert(
                PathBuf::from(index.to_string()),
                Arc::new(Listing::default()),
            );
        }
        assert_eq!(cache.items.len(), CACHE_DIRECTORIES);
        assert!(cache.get(Path::new("0")).is_none());
        assert!(cache.get(Path::new("99")).is_some());
    }

    #[test]
    fn deep_view_restores_local_selection_and_never_pollutes_cache() {
        let mut model = mixed_model();
        model.move_selection(2);
        let local = model.selected_path();
        let original = Arc::clone(model.listing.as_ref().unwrap());
        model.start_deep();
        model.push_query("api");
        let target = model.cwd.join("server/api");
        model.apply_deep(Progress {
            listing: listing(&target, &["nested"]),
            done: true,
            ..Progress::default()
        });
        model.leave_deep();
        assert_eq!(model.selected_path(), local);
        assert!(model.query.is_empty());
        assert!(Arc::ptr_eq(model.listing.as_ref().unwrap(), &original));
        let cwd = model.cwd.clone();
        assert!(Arc::ptr_eq(&model.cached(&cwd).unwrap(), &original));
    }

    #[test]
    fn query_edits_clear_deep_targets_and_change_revision_immediately() {
        let mut model = mixed_model();
        model.push_query("main");
        model.start_deep();
        let first = model.deep_query().unwrap();
        model.apply_deep(Progress {
            listing: listing(&model.cwd, &["main/api"]),
            ..Progress::default()
        });
        assert!(model.selected_directory().is_some());
        model.push_query("nope");
        assert!(model.selected_directory().is_none());
        assert_ne!(model.deep_query().unwrap().revision, first.revision);
        model.leave_deep();
        assert_eq!(model.query, "main");
        assert_eq!(model.selected_entry().unwrap().label, "main.rs");
    }

    #[test]
    fn entering_deep_result_retains_normal_listing_for_rollback() {
        let mut model = mixed_model();
        let root = model.cwd.clone();
        model.start_deep();
        model.push_query("api");
        model.apply_deep(Progress {
            listing: listing(&root, &["server/api"]),
            ..Progress::default()
        });
        let target = model.directory_target().unwrap();
        model.begin_navigation(target, None);
        assert!(model.deep.is_none());
        model.fail_navigation("directory disappeared".into());
        assert_eq!(model.cwd, root);
        assert_eq!(model.counts.files, 2);
        assert_eq!(model.counts.folders, 1);
    }

    #[test]
    fn deep_batches_preserve_selection_and_empty_query_means_no_work() {
        let mut model = mixed_model();
        model.start_deep();
        assert!(model.deep.as_ref().unwrap().progress.done);
        model.push_query("api");
        let root = model.cwd.clone();
        model.apply_deep(Progress {
            listing: listing(&root, &["b/api", "c/api"]),
            ..Progress::default()
        });
        model.move_selection(1);
        model.apply_deep(Progress {
            listing: listing(&root, &["a/api", "b/api", "c/api"]),
            done: true,
            ..Progress::default()
        });
        assert_eq!(model.selected_path(), Some(root.join("c/api")));
        model.clear_query();
        assert!(model.selected_path().is_none());
        assert!(model.deep.as_ref().unwrap().progress.done);
    }
}
