# Architecture

`ii` is a native Rust directory picker with shell integration, contextual file visibility, and opt-in deep folder search. It is not a general-purpose file manager. There is no service, network access, persistent index, plugin loader, database, or async runtime.

## Boundaries

| Component | Owns | Must not own |
| --- | --- | --- |
| `filter` | Pure subsequence scoring; safe display labels | Filesystem calls or shell commands |
| `paths` | Resolving starting locations under native filesystem rules | Terminal state or lossy path conversion |
| `filesystem` | Single-level scans; entry kinds; name-based file categories; workers | Selection policy or rendering |
| `deep` | Lazy, bounded, cancellable descendant-folder search | Normal navigation, file contents, persistent indexing |
| `model` | Location, selection, filtering, visibility, counts, cache, rollback, transient search view | Terminal I/O or synchronous scans |
| `ui` | Adaptive layout, semantic colors, visible rows, contextual details | Filesystem reads or file launching |
| `main` | Arguments, input, terminal lifetime, workers, output protocol | File modification operations |
| `shell/` | Transfer the selected directory to the calling shell | Evaluating selected paths as code |

The library boundary makes navigation and rendering testable without a terminal. The binary is the integration layer, not a second model implementation.

## Navigation contract

The header path is the current location. Right and Tab enter a selected directory inside the UI without exiting. Only Enter finishes at the header path and returns it for the shell to cd into. Left goes to the parent and selects the directory just exited.

Files are visible and selectable by default, but are context, not actions. `EntryKind` distinguishes directories, regular files with name-based categories, unresolved links, and special entries. Right and Tab share the same input branch and call `Model::directory_target`, which returns only a directory path and otherwise supplies an explanatory message. Preview scheduling calls `selected_directory`, so a selected file cannot trigger `read_dir(file)`. The detail pane only uses already-known labels/kinds. No file contents are read and no file is executed.

Directories sort before all other entries, including during local fuzzy filtering; scores rank matches within each group. Ctrl-F toggles `dirs_only`, also available as a starting CLI flag. It uses the existing listing without new I/O, retains the query and a still-visible selection, and otherwise selects the first visible match. Hidden entries are controlled independently. The preview follows visibility settings but does not inherit the local name filter.

Visible folder/file/other counts are cached during model rebuilding. Arrow-key movement and rendering do not traverse the full listing to count entries. The listing cache counts all entries, including files.

Navigation stores selection for up to 128 directories in the session. It never writes history. Sorted order determines first-visit selection; there is no inferred likely directory that unpredictably reorders the UI.

Transitions are provisional until their scan succeeds. Cached contents can appear immediately; a failed transition restores the last successful location. Only Enter received during a scan is kept as a pending finish. Tab and Right act on the currently available selected directory; without an available selection they are no-ops and never schedule completion. A later key cancels a pending finish rather than unexpectedly exiting later. The shell remains the final authority if a directory changes between scanning and cd.

## Resolved starting locations

The single-level worker resolves incoming locations with `paths::resolve` before scanning. Responses carry both the original request path (for stale-request rejection) and the resolved path (used by the model and every child entry). Thus `ii ..` lists and returns the parent directory rather than building paths under an unresolved `/..` suffix.

`std::path::absolute` handles ordinary inputs without filesystem canonicalization. On POSIX, remaining ParentDir components require `canonicalize` to preserve physical `symlink/..` semantics; lexical component popping could choose a different directory. Resolution runs on the worker, not in input dispatch. Windows native absolute rules avoid adding extended-length prefixes to ordinary paths. Existing absolute symlink aliases are retained when no parent resolution is necessary. This does not emulate a shell's logical `$PWD` rewriting.

## I/O and backpressure

Two long-lived workers serve normal navigation and previews independently. Each has one pending request, one result slot, and a generation counter. New requests replace obsolete pending work. Results are accepted only when generation and original request path match.

Workers scan one directory, check cancellation while visiting entries, and sort directories first. File categories use names/extensions, not executable bits or contents. Ordinary entries need no extra metadata beyond the platform's `DirEntry::file_type` behavior. Symlinks need a target-type check; aliases are retained. An unavailable target stays visible as an unresolved link, not silently discarded or falsely described as certainly missing. Sockets, FIFOs and devices are listed without opening them.

Preview reads begin after a 45 ms debounce and only when the pane is visible. They cannot occupy the navigation worker. The cache holds at most 32 listings and 100,000 entries total. Larger listings can be displayed but are not cached. Active listings/previews may exist outside the cache: this is not a hard process-memory cap. Showing files constructs/sorts more entries; directories-only visibility does not change the all-entry scanner/cache representation. Local filtering remains proportional to the active listing.

A blocked filesystem call cannot be forcibly cancelled by this safe Rust implementation. Workers are not joined at shutdown, keeping process exit responsive; the OS reclaims resources. A slow navigation call can delay later requests until it returns, while input/cancellation remain on the UI thread.

## Opt-in deep search

Ctrl-R enters a transient search view. It snapshots the local listing, query, selection and scroll while retaining the header as its root. Search results never enter the local listing cache. Escape, Left or Ctrl-R restores that snapshot. Beginning navigation first restores the local view so a failed deep jump rolls back to a normal directory listing, not a stale result list.

Each query edit clears visible search targets immediately and increments a revision. Root/query/hidden/revision changes cancel old work. Streamed batches preserve selection by original path when it remains in the result set. Tab/Right navigate into a result; Enter still confirms only the header directory. Ctrl-L retries; Ctrl-F explains that deep search is directory-only rather than changing local file visibility.

`deep::Controller` has no worker until an explicit nonempty query survives a 100 ms debounce. Disabled or empty search performs no recursive I/O or thread creation. Normal filtering never invokes traversal. After use, the existing search worker sleeps when idle. The search worker is independent of navigation and preview workers, and the deep view suppresses previews.

Traversal is breadth-first, directory-only, and bounded to 100,000 entries, 10,000 directory visits, depth 32, 4,096 queued directories, 200 top results and a cooperative 2-second time budget. Limits and skipped entries are reported explicitly. The time budget is checked between OS calls; a blocked call can exceed it. A single pending request and result mailbox coalesce obsolete work. Progress snapshots publish at most once per 50 ms plus final completion.

Known build/VCS/cache directories are pruned as documented in README. The excluded root itself may match, and starting a new search inside it is allowed. Hidden traversal is explicit. Symlink entries are not followed, preventing ordinary cycles; no target metadata is requested by deep search. This is not a security sandbox: a concurrently replaced directory can race between classification and traversal. There is no full `.gitignore` implementation, file-content read, or persistent index.

Basename matches rank ahead of incidental ancestor-name matches, followed by compact subsequence cost, depth and deterministic relative-path order. A query containing `/` matches relative path subsequences. Escaped relative labels are separate from original PathBuf targets. Actual disk activity during requested deep search can contend with other filesystem work; no claim of universal zero latency is made.

## Rendering and input

Ratatui renders through Crossterm on stderr in the alternate screen. Only visible rows are constructed. Rows without a query use a handful of spans, not one allocation per character. The renderer redraws on state changes, not an idle animation timer. Polling waits up to 250 ms idle and 8 ms while waiting for workers; key availability wakes it immediately.

Colors distinguish folders, code, configuration, documents, media, archives and unresolved links. Color is not the only cue: directories have `/`, links `↗`, unavailable links `↗!`, and wide rows have textual categories. File details repeat the category. NO_COLOR / --no-color use terminal defaults and reverse-video selection. No patched font is required. The preview appears at 90 columns; deep search uses full width for relative paths. Long labels preserve prefix and suffix within a grapheme-aware cell budget.

Backspace removes a grapheme cluster. Queries are capped at 256 Unicode scalar values. Matching uses lowercase Unicode subsequences, not normalization or locale collation. Ordinary letters remain searchable, including f, r and q. Help is modal; Ctrl-C/Ctrl-D still cancel.

## Terminal lifetime and output invariants

An RAII session owns raw mode, alternate screen, bracketed paste and cursor visibility. Drop restores them. The panic hook restores before reporting. Unix signal flags request cancellation on SIGTERM, SIGINT, SIGHUP and SIGQUIT. SIGKILL or sudden terminal destruction cannot be cleaned up.

Interactive stdin/stderr are required; stdout can be redirected. Noninteractive navigation fails before raw mode. Help, version and init need no terminal.

1. Paths remain PathBuf values. Escaped or lossy labels never drive filesystem operations; native parent resolution is distinct from display sanitization.
2. Control and directional-formatting characters in all labels are escaped. Paste is filter text, not a command.
3. Success writes one absolute directory path without an appended newline; --print0 appends NUL. Unix output preserves the chosen path's bytes.
4. Cancellation emits no path and exits 130; runtime errors use 1, argument errors 2.
5. Bash/Zsh preserve trailing newlines using a sentinel; Fish uses NUL-delimited fields. Quoted builtin cd and PowerShell LiteralPath avoid interpretation.
6. Browsed file contents are never opened; file-associated programs are never run; browsed files/directories are never modified. Devices/sockets are not preview files.

## Verification and documentation

Unit tests cover entry classification, directory-first ordering/filtering, navigation guards, visibility, counts, Unicode safety, symlinks/sockets, worker generations, resolved paths, selection restoration, rollback, cache bounds, colors/no-color and terminal sizes. Deep-search tests cover empty/inactive laziness, debounce, latest-query cancellation, ranking, pruning, limits and local-state restoration.

CLI tests invoke the binary noninteractively. Real Unix PTY tests verify keys, files/previews, relative start paths, deep jumps, stale-query invalidation, Enter-only completion, cancellation, stdout and terminal restoration. Separate shell tests cover quoting and initialization. Windows builds and PowerShell initialization tests do not replace interactive Windows Terminal validation.

`examples/render_demo.rs` renders the real UI through TestBackend using generic fixture data. Its SVG coalesces adjacent equally styled cells while preserving cell positions. CI checks docs/assets/demo.svg against the renderer. This is development-only, not screenshot/data collection in the application; terminal fonts can differ.

Benchmarks separate scanning and subsequence scoring, not end-to-end startup or keystroke latency. Claims must state build, machine, workload and method. Persistent frecency, recursive file search, inline terminal mode, mouse interaction, content previews, arbitrary ignore rules, configuration files and file operations remain outside scope.
