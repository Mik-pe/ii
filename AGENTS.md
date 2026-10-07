# Working on ii

Read README.md and docs/ARCHITECTURE.md before changing behavior. This is a small Rust directory navigator with contextual file visibility, not a file manager framework.

- Implement working code and regression tests, not placeholder features or speculative subsystems.
- Keep the model free of terminal/filesystem I/O. Keep scanning off the input thread.
- Preserve original PathBuf values, stdout's exact directory-path protocol, and quoted/literal shell cd behavior.
- Enter in normal browsing finishes at the header path and cds in the shell. Right opens a directory in browsing; Enter in deep search opens the selected result in browsing. Navigation never exits or queues completion. Keep one binding per action within each view. Files are visible/selectable but are never opened, executed, or returned as cd targets.
- Preserve directory-first order during filtering, Ctrl-F visibility toggles, selected directories, and cached counts without per-arrow full-list scans.
- Do not intercept ordinary letters such as f/q/j/k: users type them to filter.
- Keep name-based file categories distinct from claims about contents, permissions, or executability. Do not read files for classification or preview.
- Preserve selection on parent navigation and handle empty, unreadable, changing, and file-only directories.
- No destructive file operations, telemetry, daemon, persistent index, or network access in the application.
- Run cargo fmt --all -- --check; cargo clippy --locked --all-targets -- -D warnings; cargo test --locked --all-targets.
- Run real Unix terminal/shell tests for changes to input, cleanup, output, or shell integration.
- Regenerate docs/assets/demo.svg with cargo run --locked --example render_demo after UI changes. The fixture uses generic names only.
- Do not weaken tests or suppress warnings to get green checks. Report unavailable validation honestly.
- Keep changes focused, document behavior changes, and do not claim performance without measurements.
- Deep search is explicit Ctrl-R only. Never recurse from ordinary filtering/navigation or spawn its worker for an empty query. Keep budgets, latest-generation rejection, local-view restoration and Enter-only completion tested. Resolve parent-bearing start paths on workers before creating child entries.
