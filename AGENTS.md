# Working on ii

Read README.md and docs/ARCHITECTURE.md before changing behavior. This is a small Rust directory navigator, not a file manager framework.

- Implement working code and regression tests; do not add placeholder features or speculative subsystems.
- Keep the model free of terminal and filesystem I/O. Keep scanning off the input thread.
- Preserve original PathBuf values, stdout's exact path protocol, and quoted/literal shell cd behavior.
- Enter finishes at the header path; Tab finishes in the selected folder; Right only explores.
- Do not intercept ordinary letters such as q/j/k: users type them to filter.
- Preserve selection on parent navigation and handle empty, unreadable, and changing directories.
- No destructive file operations, telemetry, daemon, persistent index, or network access in the application.
- Run cargo fmt --all -- --check; cargo clippy --locked --all-targets -- -D warnings; cargo test --locked --all-targets.
- Run real Unix terminal and shell tests for changes to input, terminal cleanup, or shell integration.
- Do not weaken tests or suppress warnings just to obtain a green check. Report unavailable validation honestly.
- Keep changes focused, document behavior changes, and do not claim performance without measurements.
