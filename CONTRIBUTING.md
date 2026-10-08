# Contributing to ii

Keep the journey from prompt to destination short. A feature should remove effort from directory navigation, not grow a general-purpose file manager around it.

## Build and check

Use a current stable Rust toolchain. The declared minimum Rust version is 1.88 and has its own CI check.

```sh
cargo build --locked
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo build --locked --release
```

On Unix, also run the real-terminal and shell tests:

```sh
python3 scripts/test_pty.py target/release/ii
python3 scripts/test_shells.py
```

Install Bash, Zsh, and Fish to exercise every shell integration locally. Missing shells are reported as skipped. CI installs all three on Linux. PowerShell integration targets Windows and needs interactive validation in addition to the cross-platform Rust checks.

The optional `justfile` wraps these commands; `just` is not a runtime or build dependency.

## Change checklist

Preserve arrow semantics and selection when returning to a parent. Tab enters the selected directory in either view, and Right also enters a directory inside the browsing UI. In deep search, Enter also opens the selected result in the browsing view; in normal browsing, Enter confirms cd in the shell. Test that opening a search result stays interactive, emits no stdout path, and can be followed by cancellation. Keep Tab consistent across views and preserve the existing Right/Enter bindings. Do not use ordinary letters as navigation shortcuts while they are expected to filter. Keep slow filesystem work off the input thread and bounded: avoid automatic whole-tree scans, one thread per keypress, and unbounded queues.

Never replace an original path with its display label. Do not use `eval`, wildcard expansion, or whitespace splitting on selected paths. Exercise spaces, quotes, shell metacharacters, Unicode, invalid UTF-8 on Unix, and embedded/trailing newlines when changing the output or shell protocol.

For UX changes, inspect a narrow terminal, a wide terminal, an empty directory, a no-match filter, a failed directory read, a long path, and no-color mode. Update help, README, and tests together. The cover art is not a pixel-accurate UI specification.

Deep search must remain opt-in and directory-only. Test inactive/empty-query laziness, cancellation and stale-query rejection, local-view restoration, pruning and all traversal/result budgets. Opening a search result must never implicitly finish or confirm the search root. Include real-terminal regressions for empty/stale results, relative start paths and deep Enter followed by confirmation or cancellation.

A bug fix should include a regression test in the smallest applicable layer. Report which tests actually ran; do not describe skipped or unavailable environments as verified.

## Benchmarks

```sh
cargo bench --locked --bench navigation
```

Use release builds and report the workload and machine. Keep claims about scan time, matching time, startup, and input-to-render latency separate. Do not assert that `ii` is faster than another tool without running a fair, reproducible comparison.

## Repository layout

The architecture and invariants are documented in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md). Runtime logic lives in `src/`, shell integration in `shell/`, Rust integration tests in `tests/`, and development-only Python utilities in `scripts/`.

Dependencies are locked for reproducible application builds. Keep Cargo.lock committed. Changes should pass the standard read-only CI; there is no ongoing auto-commit or source-rewriting workflow.
