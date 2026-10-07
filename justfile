default:
    @just --list

# Check formatting, lints, and every Rust test target.
check:
    cargo fmt --all -- --check
    cargo clippy --locked --all-targets -- -D warnings
    cargo test --locked --all-targets

fmt:
    cargo fmt --all

# Run the actual TUI, not a mock.
run *args:
    cargo run --release -- {{args}}

install:
    cargo install --path . --locked

bench:
    cargo bench --locked --bench navigation

# Unix only; install Bash, Zsh, and Fish for complete shell coverage.
terminal-tests:
    cargo build --locked --release
    python3 scripts/test_pty.py target/release/ii
    python3 scripts/test_shells.py

package:
    cargo build --locked --release
    python3 scripts/package.py
