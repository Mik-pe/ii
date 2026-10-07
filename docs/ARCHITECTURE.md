# Architecture

`ii` is a directory picker with shell integration, not a general-purpose file manager. The binary is native Rust. There is no service, network request, index, plugin loader, database, or async runtime.

## Boundaries

| Component | Owns | Must not own |
| --- | --- | --- |
| `filter` | Pure subsequence scoring; safe display labels | Filesystem calls or shell commands |
| `filesystem` | Single-level scans; worker lifecycle; original paths | Selection policy or terminal rendering |
| `model` | Current location, selection, filter, bounded cache, rollback | Terminal I/O or synchronous scans |
| `ui` | Adaptive layout and visible-row rendering | Filesystem reads or shell integration |
| `main` | Arguments, input dispatch, terminal lifetime, worker results, output protocol | File modification operations |
| `shell/` | Transfer the selected path into the calling shell's working directory | Evaluating selected paths as code |

The library boundary makes navigation and rendering testable without opening a terminal. The binary is the integration layer, not a second implementation of the model.

## Navigation contract

The header path is the current location. The selected row is only a possible next location. Right enters that row; Enter finishes at the header path. Tab explicitly combines opening the selected row with finishing. Left goes to the parent and selects the row representing the directory just exited.

Navigation stores the most recent selection for up to 128 directories in the current session. It does not write history to disk. On first visit, sorted order determines the selection; there is no inferred "likely" folder that can unpredictably reorder the UI.

A directory transition is provisional until its worker scan succeeds. Cached contents can be shown immediately, but a failed transition restores the most recent successful location. An Enter received during a scan is kept as an explicit pending finish request. A subsequent key cancels that pending intention rather than unexpectedly exiting later.

A directory can change between scanning and the shell's `cd`; the shell remains the final authority and propagates a failed `cd`. No cached listing is a filesystem guarantee.

## I/O and backpressure

There are two long-lived workers: one for navigation and one for previews. Each has one pending request, one completed-result slot, and a generation counter. A new request replaces obsolete queued work. The UI accepts a result only when its generation and path match the active request.

Workers scan one directory, check cancellation while traversing entries, and sort the resulting subdirectories. Ordinary files need no target metadata. Directory symlinks retain their original alias paths and are never recursively expanded.

Preview reads begin after a 45 ms debounce and only on terminals wide enough to display the pane. This avoids reading a stream of directories as the user holds an arrow key. Preview work cannot occupy the navigation worker.

The cache keeps at most 32 listings and 100,000 entries in total; a listing larger than that is displayed but not cached. The active listing and preview can exist outside the cache, so this is not a hard process-memory cap. Large directories still incur one full scan and sort, and filter ranking is proportional to that listing.

A blocked filesystem system call cannot be forcibly cancelled by this safe Rust implementation. Worker threads are not joined during shutdown, allowing process exit to remain responsive. The OS reclaims their resources. A slow navigation call can delay later navigation requests until it returns, but input handling and cancellation remain on the UI thread.

## Rendering and input

Ratatui renders into the alternate screen through Crossterm on stderr. Only visible list rows are constructed. The renderer draws on state changes, not on an idle animation timer. Input polling waits up to 250 ms while idle and 8 ms while waiting for workers; key availability wakes the poll immediately.

The custom palette is optional. `NO_COLOR` and `--no-color` use default terminal colors with reverse-video selection. A preview appears at 90 columns; small terminals use one pane. No patched font is required.

Backspace removes a Unicode grapheme cluster. Query length is bounded at 256 Unicode scalar values to limit pasted input. The current matcher performs lowercase Unicode subsequence matching, not locale-aware collation or normalization. All ordinary letters remain searchable; `q` is not a quit command.

## Terminal lifetime

An RAII session owns raw mode, alternate screen, bracketed paste, and cursor visibility. Drop restores them. The panic hook restores the terminal before reporting the panic. Unix signal flags request orderly cancellation for SIGTERM, SIGINT, SIGHUP, and SIGQUIT. SIGKILL and sudden terminal/process destruction cannot be cleaned up.

The program requires interactive stdin and stderr but permits redirected stdout. A noninteractive invocation fails before entering raw mode. CLI help, version, and init commands do not require a terminal.

## Path and output invariants

1. Original paths remain `PathBuf` throughout navigation. Lossy or escaped labels are never used for filesystem operations.
2. Control and directional-formatting characters are escaped before rendering. Bracketed paste is filter text, never a command.
3. Success writes exactly one absolute path to stdout, without an appended newline. `--print0` appends NUL. Unix output preserves original path bytes.
4. Cancellation emits no path and exits 130. Runtime errors exit 1; invalid arguments exit 2.
5. Bash/Zsh preserve trailing newlines with a sentinel; Fish uses NUL-delimited fields. Quoted `builtin cd` and PowerShell's `-LiteralPath` avoid interpretation.
6. The application never modifies browsed files or directories.

## Verification

Rust unit tests cover the matcher, Unicode safety, directory scans, worker generations, selection restoration, rollback, cache bounds, and rendering across terminal sizes. CLI integration tests execute the binary noninteractively. `scripts/test_pty.py` uses real Unix pseudo-terminals and verifies terminal flags after exit. `scripts/test_shells.py` executes the actual shell functions against a controlled output protocol.

Benchmarks deliberately separate single-directory scanning from subsequence scoring. Neither result is an end-to-end startup or keystroke benchmark. Any performance claim should state the build profile, hardware/filesystem, workload, and measurement method.

## Scope of the initial version

Persistent frecency, recursive search, inline terminal mode, mouse interaction, configuration files, and file operations are not implemented. Windows builds and CLI tests do not substitute for interactive Windows Terminal / PowerShell validation. Broader terminal testing and distribution packaging should be validated before a stable release; changes must preserve the navigation and output contracts above.
