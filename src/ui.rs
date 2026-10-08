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

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Palette {
    #[default]
    Terminal,
    Dark,
    Light,
}

#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Style,
    pub muted: Style,
    pub selected: Style,
    pub warning: Style,
    pub border: Style,
    palette: Palette,
}

impl Theme {
    pub fn new(palette: Palette) -> Self {
        if palette == Palette::Terminal {
            Self {
                accent: Style::default().add_modifier(Modifier::BOLD),
                muted: Style::default(),
                selected: Style::default().add_modifier(Modifier::REVERSED | Modifier::BOLD),
                warning: Style::default().add_modifier(Modifier::BOLD),
                border: Style::default(),
                palette,
            }
        } else if palette == Palette::Light {
            Self {
                accent: Style::default()
                    .fg(Color::Rgb(20, 104, 77))
                    .add_modifier(Modifier::BOLD),
                muted: Style::default().fg(Color::Rgb(80, 96, 108)),
                selected: Style::default()
                    .fg(Color::Rgb(18, 66, 50))
                    .bg(Color::Rgb(211, 239, 226))
                    .add_modifier(Modifier::BOLD),
                warning: Style::default().fg(Color::Rgb(130, 70, 15)),
                border: Style::default().fg(Color::Rgb(91, 112, 115)),
                palette,
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
                palette,
            }
        }
    }

    pub fn entry_style(self, entry: &Entry) -> Style {
        if entry.is_dir() {
            return self.accent;
        }
        if self.palette == Palette::Terminal {
            return Style::default();
        }
        let color = match (self.palette, entry.kind) {
            (Palette::Light, EntryKind::File(FileKind::Code)) => Color::Rgb(34, 82, 153),
            (Palette::Light, EntryKind::File(FileKind::Config)) => Color::Rgb(120, 78, 12),
            (Palette::Light, EntryKind::File(FileKind::Document)) => Color::Rgb(61, 75, 88),
            (Palette::Light, EntryKind::File(FileKind::Media)) => Color::Rgb(109, 57, 135),
            (Palette::Light, EntryKind::File(FileKind::Archive)) => Color::Rgb(145, 64, 23),
            (Palette::Light, EntryKind::UnresolvedLink) => Color::Rgb(161, 43, 66),
            (Palette::Light, _) => Color::Rgb(80, 96, 108),
            (_, EntryKind::File(FileKind::Code)) => Color::Rgb(137, 180, 250),
            (_, EntryKind::File(FileKind::Config)) => Color::Rgb(232, 198, 122),
            (_, EntryKind::File(FileKind::Document)) => Color::Rgb(203, 213, 225),
            (_, EntryKind::File(FileKind::Media)) => Color::Rgb(211, 166, 235),
            (_, EntryKind::File(FileKind::Archive)) => Color::Rgb(235, 168, 107),
            (_, EntryKind::UnresolvedLink) => Color::Rgb(243, 139, 168),
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
        "↑↓ select  Tab / Enter open result  Esc back"
    } else if keys.width >= 85 && !file_selected {
        "↑↓ select  Tab / → open  ← up  Enter cd  Ctrl-R deep  Ctrl-F files  ? help"
    } else if file_selected && keys.width >= 52 {
        "↑↓ select  ← up  Enter cd here  Ctrl-F files  ? help"
    } else if file_selected {
        "↑↓ select  ← up  Enter cd  ? help"
    } else {
        "↑↓ select  Tab in  ← up  Enter cd  ? help"
    };
    frame.render_widget(Paragraph::new(hints).style(theme.muted), keys);
    if model.help {
        draw_help(frame, area, model, theme);
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
        let entries = listing
            .entries
            .iter()
            .filter(|entry| model.preview_visible(entry))
            .take(inner.height as usize)
            .collect::<Vec<_>>();
        if entries.is_empty() {
            lines.push(Line::styled(
                if model.dirs_only {
                    "No subfolders · Ctrl-F shows files"
                } else {
                    "No visible entries"
                },
                theme.muted,
            ));
        } else {
            let capacity = inner.height.saturating_sub(1) as usize;
            // One extra visible entry proves overflow without scanning the entire listing.
            let overflow = entries.len() > capacity;
            let shown = capacity.saturating_sub(usize::from(overflow));
            for entry in entries.into_iter().take(shown) {
                lines.push(row(entry, "", theme, inner.width as usize));
            }
            if overflow {
                let notice = Line::styled("… more entries", theme.muted);
                if inner.height == 1 {
                    lines[0] = notice;
                } else {
                    lines.push(notice);
                }
            }
        }
    } else if model.preview_loading {
        lines.push(Line::styled("Reading…", theme.muted));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn help_footer_height(width: u16) -> u16 {
    if width >= 42 { 1 } else { 2 }
}

pub fn help_page_size(width: u16, height: u16) -> u16 {
    let inner_width = width.saturating_sub(2).min(74).saturating_sub(2);
    height
        .saturating_sub(2)
        .min(22)
        .saturating_sub(2 + help_footer_height(inner_width))
        .max(1)
}

fn help_lines(width: usize) -> Vec<String> {
    let text = [
        "↑ / ↓          Select a folder or file",
        "Tab            Open selected folder or deep-search result",
        "→              Open selected folder (never a file)",
        "←              Parent (keep the folder selected)",
        "Enter          cd here; in deep search, open result",
        "Type           Fuzzy-filter folders and files",
        "Backspace      Erase a grapheme from the filter",
        "Esc            In browsing: clear filter, then cancel",
        "Ctrl-C         Cancel immediately",
        "Ctrl-F         Show / hide files (directories stay first)",
        "Ctrl-R         Deep folder search; Esc returns to browsing",
        ".              Toggle hidden entries when filter is empty",
        "Ctrl-L         Refresh; keep filter, selection and scroll",
        "Ctrl-G         Home directory",
        "Home / End     First / last entry",
        "PageUp / Down  Move by a page",
        "?              Open help when filter is empty",
        "",
        "File categories are name-based. No file is opened or executed.",
    ];
    let mut lines = Vec::new();
    for text in text {
        if UnicodeWidthStr::width(text) <= width {
            lines.push(text.to_owned());
            continue;
        }
        let mut line = String::new();
        for word in text.split_whitespace() {
            if !line.is_empty()
                && UnicodeWidthStr::width(line.as_str()) + 1 + UnicodeWidthStr::width(word) > width
            {
                lines.push(std::mem::take(&mut line));
            }
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
        lines.push(line);
    }
    lines
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, model: &mut Model, theme: Theme) {
    let width = area.width.saturating_sub(2).min(74);
    let height = area.height.saturating_sub(2).min(22);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.accent)
        .title(" ii · help ");
    let inner = block.inner(popup);
    let [content, footer] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(help_footer_height(inner.width)),
    ])
    .areas(inner);
    let lines = help_lines(content.width as usize);
    model.help_offset = model
        .help_offset
        .min(lines.len().saturating_sub(content.height as usize));
    frame.render_widget(Clear, popup);
    frame.render_widget(block, popup);
    frame.render_widget(
        Paragraph::new(lines[model.help_offset..].join("\n")),
        content,
    );
    frame.render_widget(
        Paragraph::new(if footer.height == 1 {
            "↑↓ / PgUp PgDn scroll · Esc / ? close"
        } else {
            "↑↓ scroll\nEsc / ? close"
        })
        .style(theme.accent),
        footer,
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
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), true))
                .unwrap();
            if width >= 20 && height >= 9 {
                assert!(text(&terminal).contains("app/"));
            }
            model.help = true;
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), true))
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
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), false))
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
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), false))
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
            .map(|entry| Theme::new(Palette::Dark).entry_style(entry).fg)
            .collect();
        assert_ne!(colors[0], colors[1]);
        assert_ne!(colors[1], colors[2]);
        assert_ne!(colors[2], colors[3]);
        model.move_selection(2);
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), true))
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
    fn light_palette_has_readable_text_on_white_and_off_white() {
        fn luminance(color: Color) -> f64 {
            let Color::Rgb(r, g, b) = color else {
                panic!("expected RGB")
            };
            let linear = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
        }
        let theme = Theme::new(Palette::Light);
        let mut styles = vec![theme.accent, theme.muted, theme.warning];
        for name in [
            "main.rs",
            "Cargo.toml",
            "README.md",
            "photo.png",
            "backup.zip",
            "other",
        ] {
            styles.push(theme.entry_style(&Entry::file(PathBuf::from(name), false)));
        }
        styles.push(theme.entry_style(&Entry::with_kind(
            PathBuf::from("link"),
            true,
            EntryKind::UnresolvedLink,
        )));
        for style in styles {
            for background in [Color::Rgb(255, 255, 255), Color::Rgb(245, 245, 245)] {
                let ratio = (luminance(background) + 0.05) / (luminance(style.fg.unwrap()) + 0.05);
                assert!(ratio >= 4.5, "insufficient text contrast: {ratio}");
            }
        }
        let ratio = (luminance(theme.selected.bg.unwrap()) + 0.05)
            / (luminance(theme.selected.fg.unwrap()) + 0.05);
        assert!(ratio >= 4.5);
    }

    #[test]
    fn short_and_narrow_help_can_reach_every_instruction_with_a_fixed_close_hint() {
        for (width, height) in [(20, 9), (40, 12), (80, 12), (120, 24)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let mut model = mixed_model();
            model.help = true;
            let mut seen = String::new();
            let mut offset = 0;
            loop {
                model.help_offset = offset;
                terminal
                    .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), false))
                    .unwrap();
                let rendered = text(&terminal);
                assert!(rendered.contains("Esc / ? close"));
                seen.push_str(&rendered);
                if model.help_offset < offset {
                    break;
                }
                offset += 1;
            }
            for instruction in [
                "Ctrl-F",
                "Ctrl-R",
                "Ctrl-L",
                "Ctrl-G",
                "grapheme",
                "executed.",
            ] {
                assert!(
                    seen.contains(instruction),
                    "{instruction} unavailable at {width}x{height}"
                );
            }
            model.help_offset = usize::MAX;
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), false))
                .unwrap();
            assert!(text(&terminal).contains("executed."));
            assert!(model.help_offset < usize::MAX);
            let mut larger = Terminal::new(TestBackend::new(120, 24)).unwrap();
            larger
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), false))
                .unwrap();
            assert_eq!(model.help_offset, 0);
        }
        for width in 16..72 {
            assert!(
                help_lines(width)
                    .iter()
                    .all(|line| UnicodeWidthStr::width(line.as_str()) <= width)
            );
        }
    }

    #[test]
    fn preview_marks_overflow_after_visibility_filtering() {
        for height in [9, 12, 24] {
            let mut terminal = Terminal::new(TestBackend::new(120, height)).unwrap();
            let mut model = mixed_model();
            model.preview_path = model.selected_directory();
            model.preview = Some(Arc::new(Listing {
                entries: (0..40)
                    .map(|index| {
                        Entry::new(PathBuf::from(format!("/code/app/child-{index:02}")), false)
                    })
                    .collect(),
                skipped: 0,
            }));
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), true))
                .unwrap();
            assert!(text(&terminal).contains("… more entries"));
            model.preview = Some(Arc::new(Listing {
                entries: (0..40)
                    .map(|index| {
                        Entry::file(PathBuf::from(format!("/code/app/file-{index:02}")), false)
                    })
                    .collect(),
                skipped: 0,
            }));
            model.toggle_files();
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), true))
                .unwrap();
            assert!(!text(&terminal).contains("… more entries"));
        }
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
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), true))
            .unwrap();
        assert!(text(&terminal).contains("cover.webp"));
        model.toggle_files();
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), true))
            .unwrap();
        assert!(!text(&terminal).contains("cover.webp"));
        model.toggle_files();
        model.move_selection(2);
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), true))
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
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), false))
            .unwrap();
        assert!(!text(&terminal).contains('\x1b'));
        assert!(text(&terminal).contains("ii"));
    }

    #[test]
    fn tab_hint_is_visible_at_narrow_and_wide_sizes_and_hidden_for_files() {
        for width in [40, 80, 85, 120] {
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            let mut model = mixed_model();
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), false))
                .unwrap();
            assert!(text(&terminal).contains("Tab"));
            model.move_selection(1);
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Terminal), false))
                .unwrap();
            assert!(!text(&terminal).contains("Tab"));
        }
    }

    #[test]
    fn deep_view_has_relative_paths_and_explicit_limit_and_completion_hints() {
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        let mut model = mixed_model();
        model.start_deep();
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
            .draw(|frame| draw(frame, &mut model, Theme::new(Palette::Dark), false))
            .unwrap();
        let rendered = text(&terminal);
        assert!(rendered.contains("DEEP SEARCH"));
        assert!(rendered.contains("server/services/api"));
        assert!(rendered.contains("limited"));
        assert!(rendered.contains("Tab / Enter open result"));
    }
}
