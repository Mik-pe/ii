//! Render the real UI with deterministic, generic data as an SVG for documentation.
//! cargo run --locked --example render_demo > docs/assets/demo.svg
use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::Arc;
use ii::filesystem::{Entry, Listing};
use ii::model::Model;
use ii::ui::{self, Theme};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};

fn escape(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn color(color: Color, fallback: &str) -> String {
    match color {
        Color::Rgb(r, g, b) => format!("#{r:02x}{g:02x}{b:02x}"),
        _ => fallback.to_owned(),
    }
}

fn main() -> io::Result<()> {
    let root = PathBuf::from("/home/dev/code/app");
    let mut model = Model::new(root.clone(), false);
    let mut entries: Vec<_> = ["assets", "docs", "src"].iter().map(|name| Entry::new(root.join(name), false)).collect();
    entries.extend(["backup.tar.gz", "Cargo.lock", "Cargo.toml", "cover.webp", "README.md"].iter().map(|name| Entry::file(root.join(name), false)));
    model.complete_navigation(Arc::new(Listing { entries, skipped: 0 }));
    model.move_selection(2);
    model.preview_path = model.selected_directory();
    model.preview = Some(Arc::new(Listing { entries: vec![
        Entry::new(root.join("src/core"), false),
        Entry::new(root.join("src/ui"), false),
        Entry::file(root.join("src/lib.rs"), false),
        Entry::file(root.join("src/main.rs"), false),
    ], skipped: 0 }));
    let mut terminal = Terminal::new(TestBackend::new(120, 24))?;
    terminal.draw(|frame| ui::draw(frame, &mut model, Theme::new(false), true))?;
    let mut out = io::BufWriter::new(io::stdout().lock());
    writeln!(out, "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1200\" height=\"480\" viewBox=\"0 0 1200 480\" role=\"img\" aria-label=\"ii file and directory navigation\">")?;
    writeln!(out, "<rect width=\"1200\" height=\"480\" rx=\"12\" fill=\"#10181a\"/>")?;
    writeln!(out, "<g font-family=\"monospace\" font-size=\"15\">")?;
    for (index, cell) in terminal.backend().buffer().content.iter().enumerate() {
        let x = (index % 120) * 10;
        let y = (index / 120) * 20;
        if cell.bg != Color::Reset {
            writeln!(out, "<rect x=\"{x}\" y=\"{y}\" width=\"10\" height=\"20\" fill=\"{}\"/>", color(cell.bg, "#10181a"))?;
        }
        if cell.symbol() != " " {
            let weight = if cell.modifier.contains(Modifier::BOLD) { 700 } else { 400 };
            writeln!(out, "<text x=\"{x}\" y=\"{}\" fill=\"{}\" font-weight=\"{weight}\">{}</text>", y + 16, color(cell.fg, "#cbd5e1"), escape(cell.symbol()))?;
        }
    }
    writeln!(out, "</g></svg>")?;
    out.flush()
}
