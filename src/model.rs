//! Pure navigation state. No terminal I/O and no synchronous filesystem calls.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use unicode_segmentation::UnicodeSegmentation;

use crate::filesystem::{Entry, Listing};
use crate::filter;

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
}

pub struct Model {
    pub cwd: PathBuf,
    pub listing: Option<Arc<Listing>>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub offset: usize,
    pub query: String,
    pub show_hidden: bool,
    pub loading: bool,
    pub message: Option<String>,
    pub help: bool,
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
            listing: None,
            visible: Vec::new(),
            selected: 0,
            offset: 0,
            query: String::new(),
            loading: true,
            message: None,
            help: false,
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

    pub fn selected_entry(&self) -> Option<&Entry> {
        let index = *self.visible.get(self.selected)?;
        self.listing.as_ref()?.entries.get(index)
    }

    pub fn selected_path(&self) -> Option<PathBuf> {
        self.selected_entry().map(|entry| entry.path.clone())
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
        self.remember();
        if !self.loading
            && let Some(listing) = &self.listing
        {
            self.fallback = Some(Location {
                path: self.cwd.clone(),
                listing: Arc::clone(listing),
                selected: self.selected_path(),
            });
        }
        self.preferred = preferred.or_else(|| {
            self.memory
                .iter()
                .find(|(candidate, _)| candidate == &path)
                .map(|(_, selected)| selected.clone())
        });
        self.cwd = path;
        self.listing = self.cache.get(&self.cwd);
        self.loading = true;
        self.message = None;
        self.query.clear();
        self.offset = 0;
        self.rebuild(self.preferred.clone());
    }

    pub fn complete_navigation(&mut self, listing: Arc<Listing>) {
        let preferred = self.selected_path().or_else(|| self.preferred.take());
        self.cache.insert(self.cwd.clone(), Arc::clone(&listing));
        self.listing = Some(listing);
        self.loading = false;
        self.fallback = None;
        self.rebuild(preferred);
    }

    pub fn fail_navigation(&mut self, error: String) {
        if let Some(previous) = self.fallback.take() {
            self.cwd = previous.path;
            self.listing = Some(previous.listing);
            self.query.clear();
            self.rebuild(previous.selected);
        } else {
            self.listing = None;
            self.visible.clear();
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
        self.visible.clear();
        if let Some(listing) = &self.listing {
            let query = self.query.to_lowercase();
            let reveal_hidden = self.show_hidden || query.starts_with('.');
            if query.is_empty() {
                self.visible.extend(
                    listing
                        .entries
                        .iter()
                        .enumerate()
                        .filter(|(_, entry)| reveal_hidden || !entry.hidden)
                        .map(|(index, _)| index),
                );
            } else {
                let mut ranked: Vec<_> = listing
                    .entries
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| reveal_hidden || !entry.hidden)
                    .filter_map(|(index, entry)| {
                        filter::score(&entry.folded, &query).map(|score| (score, index))
                    })
                    .collect();
                ranked.sort_unstable();
                self.visible
                    .extend(ranked.into_iter().map(|(_, index)| index));
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
}
