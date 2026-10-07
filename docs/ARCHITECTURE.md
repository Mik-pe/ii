# Architecture

`ii` is a directory picker with shell integration and contextual file visibility, not a general-purpose file manager. The binary is native Rust. There is no service, network request, index, plugin loader, database, or async runtime.

## Boundaries

| Component | Owns | Must not own |
| --- | --- | --- |
| `filter` | Pure subsequence scoring; safe display labels | Filesystem calls or shell commands |
| `filesystem` | Single-level scans; entry kinds; name-based file categories; workers | Selection policy or rendering |
| `model` | Location, selection, filtering, visibility, counts, cache, rollback | Terminal I/O or synchronous scans |
| `ui` | Adaptive layout, semantic colors, visible rows, contextual details | Filesystem reads or file launching |
| `main` | Arguments, input, terminal lifetime, workers, output protocol | File modification operations |
| `shell/` | Transfer the selected directory to the calling shell | Evaluating selected paths as code |

The library boundary makes navigation and rendering testable without a terminal. The binary is the integration layer, not a second model implementation.

## Navigation contract

The header path is the current location. Right enters a selected directory; Enter finishes at the header path. Tab combines opening a selected directory with finishing. Left goes to the parent and selects the directory just exited.

Files are visible and selectable by default, but are context, not actions. `EntryKind` distinguishes directories, regular files with name-based categories, unresolved links, and special entries. Both Right and delayed Tab call `Model::directory_target`, which returns only a directory path and otherwise supplies an explanatory message. Preview scheduling calls `selected_directory`, so a selected file cannot trigger `read_dir(file)`. The detail pane only uses already-known labels/kinds. No file contents are read and no file is executed.

Directories sort before all other entries, including during fuzzy filtering; scores rank matches within each group. Ctrl-F toggles `dirs_only`, also available as a starting CLI flag. It uses the existing listing without new I/O, retains the query and a still-visible selection, and otherwise selects the first visible match. Hidden entries are controlled independently. The preview pane follows the visibility settings but does not inherit the current directory's name filter.

Visible folder/file/other counts are cached during model rebuilding. Arrow-key movement and rendering do not traverse the full listing to count entries. The listing cache counts all entries, including files.

Navigation stores selection for up to 128 directories in the session. It never writes history. Sorted order determines first-visit selection; there is no inferred likely directory that unpredictably reorders the UI.

Transitions are provisional until their scan succeeds. Cached contents can appear immediately; a failed transition restores the last successful location. Enter/Tab received during a scan is kept as a pending finish. A later key cancels that pending intention instead of unexpectedly exiting later. Directories can change between scanning and the shell's `cd`; the shell remains the final authority.

## I/O and backpressure

Two long-lived workers serve navigation and previews independently. Each has one pending request, one result slot, and a generation counter. New requests replace obsolete pending work. The UI accepts results only when generation and path match the current request.

Workers scan one directory, check cancellation while visiting entries, and sort directories first. File categories use names/extensions only, not executable bits or contents. Ordinary entries need no extra metadata call beyond the platform's `DirEntry::file_type` behavior. Symlinks need a target-type check; original alias paths are retained, and no tree is recursively expanded. An unavailable target stays visible as an unresolved link, not silently discarded or falsely described as certainly missing. Sockets, FIFOs, and devices are listed without opening them.

Preview reads begin after a 45 ms debounce, only when the terminal is wide enough to show the pane. A separate worker prevents preview I/O from occupying the navigation worker.

The cache holds at most 32 listings and 100,000 entries total. Larger listings can be displayed but are not cached. Active listings and previews can exist outside the cache: this is not a hard process-memory limit. Showing files requires constructing/sorting more entries; directories-only visibility does not change the scanner's all-entry cache representation. Filtering remains proportional to the active listing.

A blocked filesystem system call cannot be forcibly cancelled by this safe Rust implementation. Workers are not joined during shutdown, keeping exit responsive; the OS reclaims their resources. A slow navigation call can delay later requests until it returns, while input and cancellation remain on the UI thread.

## Rendering and input

Ratatui renders the alternate screen through Crossterm on stderr. Only visible rows are constructed. Rows without a query use a handful of spans rather than one allocation per character. The renderer redraws on state changes, not an idle animation timer. Polling waits up to 250 ms idle and 8 ms while waiting for workers; key availability wakes it immediately.

Colors distinguish directories, code, configuration, documents, media, archives, and unresolved links. Color is not the only cue: directories have `/`, links have `↗`, unavailable links have `↗!`, and sufficiently wide rows have textual category labels. File details repeat the category. No patched font is needed. NO_COLOR / --no-color use default terminal colors and reverse-video selection. The preview/detail pane appears at 90 columns. Long names retain prefix and suffix within a grapheme-aware cell budget.

Backspace removes one grapheme cluster. Queries are capped at 256 Unicode scalar values. Matching is lowercase Unicode subsequence matching, not normalization or locale collation. All ordinary letters remain searchable, including f and q. Help is modal; Ctrl-C/Ctrl-D still cancel.

## Terminal lifetime

An RAII session owns raw mode, alternate screen, bracketed paste, and cursor visibility. Drop restores them. The panic hook restores before reporting. Unix signal flags request cancellation on SIGTERM, SIGINT, SIGHUP, and SIGQUIT. SIGKILL or sudden terminal destruction cannot be cleaned up.

Interactive stdin/stderr are required; stdout can be redirected. Noninteractive navigation fails before entering raw mode. Help, version, and init need no terminal.

## Path and output invariants

1. Original paths remain PathBuf values. Escaped or lossy labels never drive filesystem operations.
2. Control and directional-formatting characters in all entry labels are escaped. Paste is filter text, not a command.
3. Success writes one absolute directory path with no appended newline; --print0 appends NUL. Unix output preserves original bytes.
4. Cancellation emits no path and exits 130; runtime errors use 1, argument errors 2.
5. Bash/Zsh preserve trailing newlines using a sentinel; Fish uses NUL-delimited fields. Quoted builtin cd and PowerShell LiteralPath avoid interpretation.
6. The application never opens file contents, runs file-associated programs, or modifies browsed files/directories. A socket/device is never treated as a regular preview file.

## Verification and documentation

Unit tests cover entry classification, directory-first sorting/filtering, navigation guards, visibility toggles, counts, Unicode safety, symlinks/sockets, worker generations, selection restoration, rollback, cache bounds, colors/no-color, and terminal sizes. CLI tests invoke the binary noninteractively. Real Unix PTY tests verify key events, file visibility/guards, previews, output, cancellation, and terminal restoration. Separate shell tests cover quoting and initialization.

`examples/render_demo.rs` renders the real UI through TestBackend into an SVG using deterministic generic data. It is development-only and performs no screenshot/data collection in the application. Keep docs/assets/demo.svg synchronized when changing presentation; the terminal's actual font can differ from the SVG.

Benchmarks separate scanning and subsequence scoring. Neither measures end-to-end startup or keypress latency; performance claims must state their build, machine, filesystem/workload, and method.

## Deliberately outside scope

Persistent frecency, recursive search, inline terminal mode, mouse interaction, file-content previews, configuration files, and file operations are not implemented. Windows builds and PowerShell init tests do not replace interactive Windows Terminal validation. Preserve the navigation/output contracts above when extending the program.
