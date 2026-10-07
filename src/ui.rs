//! Responsive, viewport-only rendering. Ordinary Unicode; no Nerd Font required.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

use crate::filesystem::Entry;
use crate::filter::safe_label;
use crate::model::Model;

#[derive(Clone, Copy)]
pub struct Theme {
    pub accent: Style,
    pub muted: Style,
    pub selected: Style,
    pub warning: Style,
    pub border: Style,
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
            }
        }
    }
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

fn row(entry: &Entry, query: &str, theme: Theme) -> Line<'static> {
    let mut wanted = query.chars().flat_map(char::to_lowercase).peekable();
    let mut spans = Vec::with_capacity(entry.label.len().min(64) + 2);
    for ch in entry.label.chars() {
        let matches = wanted
            .peek()
            .is_some_and(|next| ch.to_lowercase().any(|candidate| candidate == *next));
        if matches {
            wanted.next();
            spans.push(Span::styled(ch.to_string(), theme.accent));
        } else {
            spans.push(Span::raw(ch.to_string()));
        }
    }
    spans.push(Span::styled(
        if entry.symlink { "/ ↗" } else { "/" },
        theme.muted,
    ));
    Line::from(spans)
}

pub fn draw(frame: &mut Frame<'_>, model: &mut Model, theme: Theme, preview_enabled: bool) {
    let area = frame.area();
    if area.width < 20 || area.height < 7 {
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
    let [header, search, content, status, keys] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(2),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(2),
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
        Span::styled("type to filter", theme.muted)
    } else {
        Span::raw(tail(
            &safe_label(&model.query),
            search.width.saturating_sub(3) as usize,
        ))
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled("/ ", theme.accent), filter])),
        search,
    );

    let (folders, preview) = if preview_enabled && area.width >= 90 {
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
        .title(" FOLDERS ");
    let inner = block.inner(folders);
    let range = model.viewport(inner.height as usize);
    if model.visible.is_empty() {
        let message = if model.loading {
            "Reading directory…"
        } else if !model.query.is_empty() {
            "No matching folders. Esc clears the filter."
        } else if model.listing.is_none() {
            "Cannot read this directory. ← goes up."
        } else {
            "No folders here. Enter stays here; ← goes up."
        };
        frame.render_widget(
            Paragraph::new(message).style(theme.muted).block(block),
            folders,
        );
    } else if let Some(listing) = &model.listing {
        let rows: Vec<_> = range
            .clone()
            .map(|index| {
                ListItem::new(row(
                    &listing.entries[model.visible[index]],
                    &model.query,
                    theme,
                ))
            })
            .collect();
        let mut state = ListState::default().with_selected(Some(model.selected - range.start));
        frame.render_stateful_widget(
            List::new(rows)
                .block(block)
                .highlight_symbol("› ")
                .highlight_style(theme.selected),
            folders,
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
    } else {
        let skipped = model.listing.as_ref().map_or(0, |listing| listing.skipped);
        let mut text = format!(
            "{} / {} folders",
            if model.visible.is_empty() {
                0
            } else {
                model.selected + 1
            },
            model.visible.len()
        );
        if model.show_hidden {
            text.push_str("   · hidden on");
        }
        if skipped > 0 {
            text.push_str(&format!("   · {skipped} unreadable entries skipped"));
        }
        Line::styled(text, theme.muted)
    };
    frame.render_widget(Paragraph::new(text), status);
    let hints = if keys.width >= 85 {
        "↑↓ select   → open   ← up   Enter cd here   Tab cd selected   . hidden   ? help"
    } else {
        "↑↓ select  → open  ← up  Enter cd  Esc cancel  ? help"
    };
    frame.render_widget(Paragraph::new(hints).style(theme.muted), keys);
    if model.help {
        draw_help(frame, area, theme);
    }
}

fn draw_preview(frame: &mut Frame<'_>, area: Rect, model: &Model, theme: Theme) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme.border)
        .title(" NEXT → ");
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let inner = inner.inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    let Some(path) = &model.preview_path else {
        frame.render_widget(
            Paragraph::new("Select a folder to see what is inside.").style(theme.muted),
            inner,
        );
        return;
    };
    let mut lines = vec![Line::styled(
        tail(
            &safe_label(
                &path
                    .file_name()
                    .unwrap_or(path.as_os_str())
                    .to_string_lossy(),
            ),
            inner.width as usize,
        ),
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
            .filter(|entry| model.show_hidden || !entry.hidden)
            .peekable();
        if entries.peek().is_none() {
            lines.push(Line::styled("No subfolders", theme.muted));
        } else {
            for entry in entries.take(inner.height.saturating_sub(1) as usize) {
                lines.push(Line::styled(
                    format!(
                        "  {}/{}",
                        tail(&entry.label, inner.width.saturating_sub(5) as usize),
                        if entry.symlink { " ↗" } else { "" }
                    ),
                    theme.muted,
                ));
            }
        }
    } else if model.preview_loading {
        lines.push(Line::styled("Reading…", theme.muted));
    }
    frame.render_widget(Paragraph::new(Text::from(lines)), inner);
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, theme: Theme) {
    let width = area.width.saturating_sub(2).min(70);
    let height = area.height.saturating_sub(2).min(20);
    let popup = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let text = [
        "↑ / ↓          Select a folder",
        "→              Open selected folder",
        "←              Parent (keep the folder selected)",
        "Enter          Finish in the directory in the header",
        "Tab            Open selected folder and finish",
        "Type           Fuzzy-filter this directory (q is a letter!)",
        "Backspace      Erase one character; parent if filter is empty",
        "Esc            Clear filter; otherwise cancel without cd",
        "Ctrl-C / Ctrl-D Cancel immediately",
        ".              Toggle hidden folders when filter is empty",
        "Ctrl-U         Clear the filter",
        "Ctrl-L         Refresh the current directory",
        "Ctrl-G         Home directory",
        "Home / End     First / last folder",
        "PageUp / Down  Move by a page",
        "? / F1         Toggle this help",
        "",
        "Read-only navigation. No indexing, daemon, or file operations.",
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

    #[test]
    fn truncation_respects_unicode_cells() {
        for width in 0..20 {
            assert!(UnicodeWidthStr::width(tail("/home/日本/🧑‍💻/räv", width).as_str()) <= width);
        }
        assert_eq!(tail("short", 10), "short");
    }

    #[test]
    fn renders_tiny_normal_and_wide_terminals() {
        for (width, height) in [(1, 1), (19, 6), (20, 7), (40, 10), (80, 24), (120, 30)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let root = PathBuf::from("/code");
            let mut model = Model::new(root.clone(), false);
            model.complete_navigation(Arc::new(Listing {
                entries: ["app", "notes", "tools"]
                    .iter()
                    .map(|name| Entry::new(root.join(name), false))
                    .collect(),
                skipped: 0,
            }));
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(false), true))
                .unwrap();
            model.help = true;
            terminal
                .draw(|frame| draw(frame, &mut model, Theme::new(true), true))
                .unwrap();
        }
    }

    #[test]
    fn render_does_not_include_terminal_controls_from_paths() {
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        let mut model = Model::new(PathBuf::from("/code/evil\x1b[2J"), false);
        terminal
            .draw(|frame| draw(frame, &mut model, Theme::new(false), false))
            .unwrap();
        let text: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(!text.contains('\x1b'));
        assert!(text.contains("ii"));
    }
}
