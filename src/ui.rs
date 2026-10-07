//! Responsive, viewport-only rendering. Ordinary Unicode; no Nerd Font required.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::filesystem::{Entry, EntryKind, FileKind};
use crate::filter::safe_label;
use crate::model::Model;

#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Style,
    pub muted: Style,
    pub selected: Style,
    pub warning: Style,
    pub border: Style,
    no_color: bool,
}

impl Theme {
    pub fn new(no_color: bool) -> Self {
        if no_color {
            Self {
                accent: Style::default().add_modifier(Modifier::BOLD),
                muted: Style::default(),
                selected: Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                warning: Style::default().add_modifier(Modifier::BOLD),
                border: Style::default(),
                no_color,
            }
        } else {
            Self {
                accent: Style::default()
                    .fg(Color::Rgb(133, 232, 194))
                    .add_modifier(Modifier::BOLD),
                muted: Style::default().fg(Color::Rgb(142, 154, 165)),
                selected: Style::default()
                    .fg(Color::Rgb(218, 255, 240))
                    .bg(Color::Rgb(29, 69, 59))
                    .add_modifier(Modifier::BOLD),
                warning: Style::default().fg(Color::Rgb(244, 188, 111)),
                border: Style::default().fg(Color::Rgb(73, 96, 101)),
                no_color,
            }
        }
    }

    pub fn entry_style(self, entry: &Entry) -> Style {
        if entry.is_dir() {
            return self.accent;
        }
        if self.no_color {
            return Style::default();
        }
        let color = match entry.kind {
            EntryKind::File(FileKind::Code) => Color::Rgb(137, 180, 250),
            EntryKind::File(FileKind::Config) => Color::Rgb(232, 198, 122),
            EntryKind::File(FileKind::Document) => Color::Rgb(203, 213, 225),
            EntryKind::File(FileKind::Media) => Color::Rgb(211, 166, 235),
            EntryKind::File(FileKind::Archive) => Color::Rgb(235, 168, 107),
            EntryKind::UnresolvedLink => Color::Rgb(243, 139, 168),
            _ => Color::Rgb(160, 173, 185),
        };
        Style::default().fg(color)
    }
}

fn vertical_spacing(height: u16) -> u16 {
    if height < 16 { 1 } else { 2 }
}

/// Actual list-row count, shared with PageUp/PageDown input dispatch.
pub fn page_size(height: u16) -> u16 {
    height
        .saturating_sub(2)
        .min(32)
        .saturating_sub(vertical_spacing(height) * 3 + 3)
        .max(1)
}

/// Keep the useful right-hand end of a path; measure terminal cells, not bytes.
pub fn tail(input: &str, width: usize) -> String {
    if UnicodeWidthStr::width(input) <= width {
        return input.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let mut used = 1;
    let mut start = input.len();
    for (index, grapheme) in input.grapheme_indices(true).rev() {
        let cells = UnicodeWidthStr::width(grapheme);
        if used + cells > width {
            break;
        }
        used += cells;
        start = index;
    }
    format!("…{}", &input[start..])
}

/// Preserve both a filename's prefix and its extension when space is tight.
fn middle(input: &str, width: usize) -> String {
    if UnicodeWidthStr::width(input) <= width {
        return input.to_owned();
    }
    if width == 0 {
        return String::new();
    }
    let budget = (width - 1) * 2 / 3;
    let mut prefix = String::new();
    let mut used = 0;
    for grapheme in input.graphemes(true) {
        let cells = UnicodeWidthStr::width(grapheme);
        if used + cells > budget {
            break;
        }
        prefix.push_str(grapheme);
        used += cells;
    }
    prefix.push_str(&tail(input, width - used));
    prefix
}

fn row(entry: &Entry, query: &str, theme: Theme, width: usize) -> Line<'static> {
    let style = theme.entry_style(entry);
    let badge = if width >= 34 && !entry.is_dir() {
        entry.type_label()
    } else {
        ""
    };
    let badge_width = UnicodeWidthStr::width(badge);
    let suffix = entry.suffix();
    let label_width = width.saturating_sub(
        UnicodeWidthStr::width(suffix) + badge_width + if badge.is_empty() { 0 } else { 2 },
    );
    let label = middle(&entry.label, label_width);
    let used = UnicodeWidthStr::width(label.as_str()) + UnicodeWidthStr::width(suffix);
    let mut spans = Vec::new();
    if query.is_empty() {
        // The hot arrow-key path allocates a handful of spans, not one per character.
        spans.push(Span::styled(label, style));
    } else {
        let mut wanted = query.chars().flat_map(char::to_lowercase).peekable();
        for ch in label.chars() {
            let matched = wanted
                .peek()
                .is_some_and(|next| ch.to_lowercase().any(|candidate| candidate == *next));
            if matched {
                wanted.next();
            }
            let style = if matched {
                style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
            } else {
                style
            };
            spans.push(Span::styled(ch.to_string(), style));
        }
    }
    spans.push(Span::styled(suffix, style));
    if !badge.is_empty() {
        spans.push(Span::raw(
            " ".repeat(width.saturating_sub(used + badge_width)),
        ));
        spans.push(Span::styled(badge, style));
    }
    Line::from(spans)
}

pub fn draw(frame: &mut Frame<'_>, model: &mut Model, theme: Theme, preview_enabled: bool) {
    let area = frame.area();
    if area.width < 20 || area.height < 9 {
        frame.render_widget(Paragraph::new("ii · enlarge terminal\nEsc cancels"), area);
        return;
    }
    let width = area.width.saturating_sub(4).min(144);
    let body = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + 1,
        width,
        area.height.saturating_sub(2).min(32),
    );
    let spacing = vertical_spacing(area.height);
    let [header, search, content, status, keys] = Layout::vertical([
        Constraint::Length(spacing),
        Constraint::Length(spacing),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(spacing),
    ])
    .areas(body);
    let path = tail(
        &safe_label(&model.cwd.to_string_lossy()),
        header.width.saturating_sub(8) as usize,
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("ii", theme.accent),
            Span::raw("   "),
            Span::raw(path),
        ])),
        header,
    );
    let filter = if model.query.is_empty() {
        Span::styled(
            if model.deep.is_some() {
                "type a folder name or relative path"
            } else {
                "type to filter"
            },
            theme.muted,
        )
    } else {
        Span::raw(tail(
            &safe_label(&model.query),
            search.width.saturating_sub(3) as usize,
        ))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                if model.deep.is_some() { "// " } else { "/ " },
                theme.accent,
            ),
            filter,
        ])),
        search,
    );

    let (entries, preview) = if preview_enabled && area.width >= 90 && model.deep.is_none() {
        let [left, right] =
            Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)])
                .areas(content);
        (left, Some(right))
    } else {
        (content, None)
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border)
        .title(if model.deep.is_some() {
            " DEEP SEARCH · folders "
        } else if model.dirs_only {
            " FOLDERS "
        } else {
            " CONTENTS "
        });
    let inner = block.inner(entries);
    let range = model.viewport(inner.height as usize);
    if model.visible.is_empty() {
        let message = if model.loading {
            "Reading directory…"
        } else if let Some(deep) = &model.deep {
            if model.query.is_empty() {
                "Type to search below this folder. Esc goes back."
            } else if !deep.progress.done {
                "Searching below this folder…"
            } else {
                "No matching subfolders. Ctrl-L retries; Esc goes back."
            }
        } else if !model.query.is_empty() {
            "No matches. Esc clears the filter."
        } else if model.listing.is_none() {
            "Cannot read this directory. ← goes up."
        } else if model.dirs_only {
            "No folders. Ctrl-F shows files; Enter stays here."
        } else {
            "No visible entries. Enter stays here; . shows hidden."
        };
        frame.render_widget(
            Paragraph::new(message).style(theme.muted).block(block),
            entries,
        );
    } else if let Some(listing) = &model.listing {
        let rows: Vec<_> = range
            .clone()
            .map(|index| {
                ListItem::new(row(
                    &listing.entries[model.visible[index]],
                    &model.query,
                    theme,
                    inner.width.saturating_sub(2) as usize,
                ))
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(model.selected - range.start));
        frame.render_stateful_widget(
            List::new(rows)
                .block(block)
                .highlight_symbol("› ")
                .highlight_style(theme.selected),
            entries,
            &mut state,
        );
    }
    if let Some(area) = preview {
        draw_preview(frame, area, model, theme);
    }

    let text = if let Some(message) = &model.message {
        Line::styled(tail(message, status.width as usize), theme.warning)
    } else if model.loading {
        Line::styled("Reading directory…  Esc cancels", theme.muted)
    } else if let Some(deep) = &model.deep {
        let progress = &deep.progress;
        if let Some(error) = &progress.error {
            Line::styled(tail(error, status.width as usize), theme.warning)
        } else {
            let state = if !progress.done {
                "searching"
            } else if progress.limited {
                "limited · narrow scope"
            } else {
                "done"
            };
            Line::styled(
                format!(
                    "{state} · {} folders · {} checked · {} skipped",
                    model.visible.len(),
                    progress.visited,
                    progress.skipped
                ),
                theme.muted,
            )
        }
    } else {
        let selected = if model.visible.is_empty() {
            0
        } else {
            model.selected + 1
        };
        let mut text = format!(
            "{selected}/{}   {} folders · {} files",
            model.visible.len(),
            model.counts.folders,
            model.counts.files
        );
        if model.counts.other > 0 {
            text.push_str(&format!(" · {} other", model.counts.other));
        }
        if model.dirs_only {
            text.push_str(" · files hidden");
        }
        if model.show_hidden {
            text.push_str(" · hidden on");
        }
        let skipped = model.listing.as_ref().map_or(0, |listing| listing.skipped);
        if skipped > 0 {
            text.push_str(&format!(" · {skipped} unreadable entries skipped"));
        }
        Line::styled(text, theme.muted)
    };
    frame.render_widget(Paragraph::new(text), status);
    let file_selected = model.selected_entry().is_some_and(|entry| !entry.is_dir());
    let hints = if model.deep.is_some() {
        "↑↓ select  Tab/→ open result  Enter cd here  Esc back"
    } else if keys.width >= 85 && !file_selected {
        "↑↓ select  →/Tab open  ← up  Enter cd  Ctrl-R deep  Ctrl-F files  ? help"
    } else if file_selected && keys.width >= 52 {
        "↑↓ select  ← up  Enter cd here  Ctrl-F files  ? help"
    } else if file_selected {
        "↑↓ select  ← up  Enter cd  ? help"
    } else {
        "↑↓ select  →/Tab in ← up  Enter cd  ? help"
    };
    frame.render_widget(Paragraph::new(hints).style(theme.muted), keys);
    if model.help {
        draw_help(frame, area, theme);
    }
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, model: &Model, theme: Theme) {
    let selected = model.selected_entry();
    let detail = selected.is_some_and(|entry| !entry.is_dir());
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border)
        .title(if detail { " DETAILS " } else { " NEXT → " });
    let inner = block.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    frame.render_widget(block, area);
    if let Some(entry) = selected.filter(|entry| !entry.is_dir()) {
        // Deliberately no content reads, stat calls, or external viewers here.
        let mut lines = vec![
            Line::styled(
                middle(&entry.label, inner.width as usize),
                theme.entry_style(entry),
            ),
            Line::styled(entry.type_label(), theme.entry_style(entry)),
        ];
        if entry.symlink {
            lines.push(Line::styled("Symbolic link ↗", theme.muted));
        }
        lines.push(Line::raw(""));
        if entry.kind == EntryKind::UnresolvedLink {
            lines.push(Line::styled("Target unavailable", theme.warning));
        } else if matches!(entry.kind, EntryKind::File(_)) {
            lines.push(Line::styled(
                "Category inferred from filename.",
                theme.muted,
            ));
        }
        lines.extend([
            Line::styled("Not opened or executed.", theme.muted),
            Line::raw(""),
            Line::raw("Enter   cd to this folder"),
            Line::raw("Ctrl-F  show folders only"),
        ]);
        frame.render_widget(Paragraph::new(Text::from(lines)), inner);
        return;
    }
    let Some(path) = &model.preview_path else {
        frame.render_widget(
            Paragraph::new("Select a folder to see inside.").style(theme.muted),
            inner,
        );
        return;
    };
    let label = safe_label(
        &path
            .file_name()
            .unwrap_or(path.as_os_str())
            .to_string_lossy(),
    );
    let mut lines = vec![Line::styled(
        middle(&label, inner.width as usize),
        theme.accent,
    )];
    if let Some(error) = &model.preview_error {
        lines.push(Line::styled(
            tail(error, inner.width as usize),
            theme.warning,
        ));
    } else if let Some(listing) = &model.preview {
        let mut entries = listing
            .entries
            .iter()
            .filter(|entry| model.preview_visible(entry))
            .peekable();
        if entries.peek().is_none() {
            lines.push(Line::styled(
                if model.dirs_only {
                    "No subfolders · Ctrl-F shows files"
                } else {
                    "No visible entries"
                },
                theme.muted,
            ));
        } else {
            for entry in entries.take(inner.height.saturating_sub(1) as usize) {
                lines.push(row(entry, "", theme, inner.width as usize));
            }
        }
    } else if model.preview_loading {
        lines.push(Line::styled("Reading…", theme.muted));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let width = area.width.saturating_sub(2).min(74);
    let height = area.height.saturating_sub(2).min(22);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let text = [
        "↑ / ↓          Select a folder or file",
        "→              Open selected folder (never a file)",
        "←              Parent (keep the folder selected)",
        "Enter          Finish in the directory in the header",
        "Tab            Open selected folder (stay in UI)",
        "Type           Fuzzy-filter folders and files",
        "Backspace      Erase a character; parent if filter is empty",
        "Esc            Clear filter; otherwise cancel without cd",
        "Ctrl-C / Ctrl-D Cancel immediately",
        "Ctrl-F         Show / hide files (directories stay first)",
        "Ctrl-R         Deep folder search; Esc returns to browsing",
        ".              Toggle hidden entries when filter is empty",
        "Ctrl-U         Clear the filter",
        "Ctrl-L         Refresh the current directory",
        "Ctrl-G         Home directory",
        "Home / End     First / last entry",
        "PageUp / Down  Move by a page",
        "? / F1         Toggle this help",
        "",
        "File categories are name-based. No file is opened or executed.",
    ];
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(text.join("\n")).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(theme.accent)
                .title(" ii · two taps, any directory "),
        ),
        popup,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::filesystem::Listing;
    use ratatui::{Terminal, backend::TestBackend};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn mixed_model() -> Model {
        let root = PathBuf::from("/code");
        let mut model = Model::new(root.clone(), false);
        model.complete_navigation(Arc::new(Listing {
            entries: vec![
                Entry::new(root.join("app"), false),
                Entry::file(root.join("Cargo.toml"), false),
                Entry::file(root.join("main.rs"), false),
                Entry::file(root.join("README.md"), false),
            ],
            skipped: 0,
        }));
        model
    }

    fn text(terminal: &Terminal<TestBackend>) -> String {
        terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn truncation_respects_unicode_cells_and_preserves_extensions() {
        for width in 0..30 {
            for input in [
                "/home/日本/🧑‍💻/räv",
                "a-very-long-source-file.rs",
                "e\u{301}🦊日本.txt",
            ] {
                assert!(UnicodeWidthStr::width(tail(input, width).as_str()) <= width);
                assert!(UnicodeWidthStr::width(middle(input, width).as_str()) <= width);
            }
        }
        assert_eq!(tail("short", 10), "short");
        let label = middle("a-very-long-source-file.rs", 15);
        assert!(label.starts_with("a-very"));
        assert!(label.ends_with(".rs"));
    }

    #[test]
    fn renders_tiny_normal_and_wide_terminals() {
        for (width, height) in [(1, 1), (19, 6), (20, 7), (40, 10), (80, 24), (120, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut model = mixed_model();
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(false), true))
                .unwrap();
            if width >= 20 && height >= 9 {
                assert!(text(&terminal).contains("app/"));
            }
            model.help = true;
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(true), true))
                .unwrap();
        }
    }

    #[test]
    fn page_size_matches_rendered_rows_at_every_supported_height() {
        for height in 9..50 {
            let mut terminal = Terminal::new(TestBackend::new(60, height)).unwrap();
            let root = PathBuf::from("/code");
            let mut model = Model::new(root.clone(), false);
            model.complete_navigation(Arc::new(Listing {
                entries: (0..100)
                    .map(|index| Entry::new(root.join(format!("folder-{index:03}")), false))
                    .collect(),
                skipped: 0,
            }));
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(true), false))
                .unwrap();
            assert_eq!(
                text(&terminal).matches("folder-").count(),
                page_size(height) as usize
            );
        }
    }

    #[test]
    fn categories_have_distinct_colors_and_survive_no_color_mode() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut model = mixed_model();
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), false))
            .unwrap();
        let rendered = text(&terminal);
        assert!(rendered.contains("Cargo.toml"));
        assert!(rendered.contains("main.rs"));
        assert!(rendered.contains("config"));
        assert!(rendered.contains("code"));
        assert!(rendered.contains("1 folders · 3 files"));
        let colors: Vec<_> = model
            .listing
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .map(|entry| Theme::new(false).entry_style(entry).fg)
            .collect();
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[1], colors[2]);
        assert_ne!(colors[2], colors[3]);
        model.move_selection(2);
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(true), true))
            .unwrap();
        assert!(text(&terminal).contains("main.rs"));
        for cell in &terminal.backend().buffer().content {
            assert_eq!(cell.fg, Color::Reset);
            assert_eq!(cell.bg, Color::Reset);
        }
        assert!(
            terminal
                .backend()
                .buffer()
                .content
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::REVERSED))
        );
    }

    #[test]
    fn next_directory_shows_files_but_file_details_never_show_stale_listing() {
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        let mut model = mixed_model();
        model.preview_path = model.selected_directory();
        model.preview = Some(Arc::new(Listing {
            entries: vec![
                Entry::new(PathBuf::from("/code/app/src"), false),
                Entry::file(PathBuf::from("/code/app/cover.webp"), false),
            ],
            skipped: 0,
        }));
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), true))
            .unwrap();
        assert!(text(&terminal).contains("cover.webp"));
        model.toggle_files();
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), true))
            .unwrap();
        assert!(!text(&terminal).contains("cover.webp"));
        model.toggle_files();
        model.move_selection(2);
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), true))
            .unwrap();
        let rendered = text(&terminal);
        assert!(rendered.contains("DETAILS"));
        assert!(rendered.contains("Not opened or executed."));
        assert!(!rendered.contains("cover.webp"));
    }

    #[test]
    fn render_does_not_include_terminal_controls_from_paths_or_files() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut model = Model::new(PathBuf::from("/code/evil\x1b[2J"), false);
        model.complete_navigation(Arc::new(Listing {
            entries: vec![Entry::file(PathBuf::from("/code/evil\x1b[2J.rs"), false)],
            skipped: 0,
        }));
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), false))
            .unwrap();
        assert!(!text(&terminal).contains('\x1b'));
        assert!(text(&terminal).contains("ii"));
    }

    #[test]
    fn deep_view_has_relative_paths_and_explicit_limit_and_completion_hints() {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let mut model = mixed_model();
        model.toggle_deep();
        model.push_query("api");
        let mut entry = Entry::new(PathBuf::from("/code/server/services/api"), false);
        entry.label = "server/services/api".into();
        entry.folded = entry.label.clone();
        model.apply_deep(crate::deep::Progress {
            listing: Arc::new(Listing {
                entries: vec![entry],
                skipped: 0,
            }),
            limited: true,
            done: true,
            ..crate::deep::Progress::default()
        });
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), false))
            .unwrap();
        let rendered = text(&terminal);
        assert!(rendered.contains("DEEP SEARCH"));
        assert!(rendered.contains("server/services/api"));
        assert!(rendered.contains("limited"));
        assert!(rendered.contains("Enter cd here"));
    }
}
