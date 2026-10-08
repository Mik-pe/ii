//! Render the real UI with deterministic, generic data as an SVG for documentation.
//! cargo run --locked --example render_demo > docs/assets/demo.svg
use ii::filesystem::{Entry, Listing};
use ii::model::Model;
use ii::ui::{self, Palette, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn color(color: Color, fallback: &str) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_owned(),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = PathBuf::from("/home/dev/code/app");
    let mut model = Model::new(root.clone(), false);
    let mut entries: Vec<_> = ["assets", "docs", "src"]
        .iter()
        .map(|name| Entry::new(root.join(name), false))
        .collect();
    entries.extend(
        [
            "backup.tar.gz",
            "Cargo.lock",
            "Cargo.toml",
            "cover.webp",
            "README.md",
        ]
        .iter()
        .map(|name| Entry::file(root.join(name), false)),
    );
    model.complete_navigation(Arc::new(Listing {
        entries,
        skipped: 0,
    }));
    model.move_selection(2);
    model.preview_path = model.selected_directory();
    model.preview = Some(Arc::new(Listing {
        entries: vec![
            Entry::new(root.join("src/core"), false),
            Entry::new(root.join("src/ui"), false),
            Entry::file(root.join("src/lib.rs"), false),
            Entry::file(root.join("src/main.rs"), false),
        ],
        skipped: 0,
    }));
    let mut terminal = Terminal::new(TestBackend::new(120, 24))?;
    terminal.draw(|frame| ui::draw(frame, &mut model, Theme::new(Palette::Dark), true))?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1200\" height=\"480\" viewBox=\"0 0 1200 480\" role=\"img\" aria-label=\"ii file and directory navigation\">"
    )?;
    writeln!(
        out,
        "<rect width=\"1200\" height=\"480\" rx=\"12\" fill=\"#10181a\"/>"
    )?;
    writeln!(out, "<g font-family=\"monospace\" font-size=\"15\">")?;
    // Coalesce adjacent cells with identical styles. This preserves the actual
    // renderer's cell positions while keeping the documentation asset compact.
    for (y, cells) in terminal.backend().buffer().content.chunks(120).enumerate() {
        let mut start = 0;
        while start < cells.len() {
            let first = &cells[start];
            let mut end = start + 1;
            while end < cells.len() && cells[end].style() == first.style() {
                end += 1;
            }
            let x = start * 10;
            let width = (end - start) * 10;
            let baseline = y * 20 + 16;
            if first.bg != Color::Reset {
                writeln!(
                    out,
                    "<rect x=\"{x}\" y=\"{}\" width=\"{width}\" height=\"20\" fill=\"{}\"/>",
                    y * 20,
                    color(first.bg, "#10181a")
                )?;
            }
            let text: String = cells[start..end].iter().map(|cell| cell.symbol()).collect();
            let text = text.trim_end_matches(' ');
            if !text.is_empty() {
                let positions = cells[start..end]
                    .iter()
                    .enumerate()
                    .flat_map(|(offset, cell)| {
                        let x = (start + offset) * 10;
                        cell.symbol().chars().map(move |_| x.to_string())
                    })
                    .take(text.chars().count())
                    .collect::<Vec<_>>()
                    .join(" ");
                let weight = if first.modifier.contains(Modifier::BOLD) {
                    700
                } else {
                    400
                };
                writeln!(
                    out,
                    "<text x=\"{positions}\" y=\"{baseline}\" fill=\"{}\" font-weight=\"{weight}\" xml:space=\"preserve\">{}</text>",
                    color(first.fg, "#cbd5e1"),
                    escape(text)
                )?;
            }
            start = end;
        }
    }
    writeln!(out, "</g></svg>")?;
    out.flush()?;
    Ok(())
}
