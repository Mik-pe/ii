use crossterm::cursor::{Hide, Show};
use crossterm::event::{
    self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers,
};
use crossterm::execute;
use crossterm::style::ResetColor;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ii::deep::{Controller, Progress};
use ii::filesystem::Scanner;
use ii::filter::safe_label;
use ii::model::Model;
use ii::ui::{self, Theme};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const HELP: &str = "ii — two taps, any directory\n\nUSAGE\n    ii [OPTIONS] [PATH]\n    ii init <bash|zsh|fish|powershell>\n\nOPTIONS\n    -a, --hidden       Show hidden entries\n        --dirs-only    Start with files hidden (Ctrl-F toggles)\n        --no-preview   Use a single directory pane\n        --no-color     Use your terminal colors (also honors NO_COLOR)\n        --print0       Terminate the selected path with NUL (for scripts)\n    -h, --help         Print this help\n    -V, --version      Print the version\n        --             Treat the remaining argument as a path\n\nKEYS\n    ↑↓ select   → open folder   ← parent   Enter cd here   Tab open folder\n    Type to filter folders and files. Ctrl-F shows/hides files. Ctrl-R searches descendant folders.\n    Files have name-based colors and type labels; they are never opened or executed.\n    Esc clears the filter, then cancels. Ctrl-C always cancels.\n    . toggles hidden entries. Ctrl-L refreshes. ? opens help.\n\nSHELL SETUP (once in your shell profile)\n    bash:       eval \"$(ii init bash)\"\n    zsh:        eval \"$(ii init zsh)\"\n    fish:       ii init fish | source\n    PowerShell: ii.exe init powershell | Out-String | Invoke-Expression\n\nPowerShell setup replaces the built-in ii alias for Invoke-Item.\nThe UI uses stderr. A successful selection writes only the raw absolute path\n(with no trailing newline) to stdout. Cancellation exits 130 with no path.\nA subprocess cannot change its parent directory: install the shell function.\n";

#[derive(Default)]
struct Options {
    path: Option<PathBuf>,
    hidden: bool,
    dirs_only: bool,
    no_preview: bool,
    no_color: bool,
    print0: bool,
}

enum Command {
    Run(Options),
    Print(&'static str),
}

fn parse(arguments: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let mut arguments = arguments.into_iter().peekable();
    if arguments.peek().is_some_and(|argument| argument == "init") {
        arguments.next();
        let shell = arguments
            .next()
            .ok_or("expected a shell: bash, zsh, fish, or powershell")?;
        if arguments.next().is_some() {
            return Err("too many arguments to init".into());
        }
        return match shell.to_str() {
            Some("bash" | "zsh") => Ok(Command::Print(include_str!("../shell/ii.sh"))),
            Some("fish") => Ok(Command::Print(include_str!("../shell/ii.fish"))),
            Some("powershell" | "pwsh") => Ok(Command::Print(include_str!("../shell/ii.ps1"))),
            _ => Err("unsupported shell; use bash, zsh, fish, or powershell".into()),
        };
    }
    let mut options = Options::default();
    let mut positional = false;
    for argument in arguments {
        if !positional {
            match argument.to_str() {
                Some("--") => {
                    positional = true;
                    continue;
                }
                Some("-h" | "--help") => return Ok(Command::Print(HELP)),
                Some("-V" | "--version") => {
                    return Ok(Command::Print(concat!(
                        "ii ",
                        env!("CARGO_PKG_VERSION"),
                        "\n"
                    )));
                }
                Some("-a" | "--hidden") => {
                    options.hidden = true;
                    continue;
                }
                Some("--dirs-only") => {
                    options.dirs_only = true;
                    continue;
                }
                Some("--no-preview") => {
                    options.no_preview = true;
                    continue;
                }
                Some("--no-color") => {
                    options.no_color = true;
                    continue;
                }
                Some("--print0") => {
                    options.print0 = true;
                    continue;
                }
                Some(flag) if flag.starts_with('-') => {
                    return Err(format!("unknown option: {}", safe_label(flag)));
                }
                _ => {}
            }
        }
        if options.path.replace(PathBuf::from(argument)).is_some() {
            return Err("expected at most one starting directory".into());
        }
    }
    Ok(Command::Run(options))
}

static TERMINAL_ACTIVE: AtomicBool = AtomicBool::new(false);

fn restore_terminal() {
    if TERMINAL_ACTIVE.swap(false, Ordering::AcqRel) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stderr(),
            DisableBracketedPaste,
            ResetColor,
            LeaveAlternateScreen,
            Show
        );
    }
}

struct Session;
impl Session {
    fn start() -> io::Result<Self> {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |information| {
            restore_terminal();
            previous(information);
        }));
        enable_raw_mode()?;
        TERMINAL_ACTIVE.store(true, Ordering::Release);
        let session = Self;
        execute!(
            io::stderr(),
            EnterAlternateScreen,
            EnableBracketedPaste,
            Hide
        )?;
        Ok(session)
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        restore_terminal();
    }
}

enum Intent {
    None,
    Navigate(PathBuf, Option<PathBuf>),
    Finish,
    Cancel,
}

fn parent(model: &Model) -> Intent {
    model.parent().map_or(Intent::None, |(path, selected)| {
        Intent::Navigate(path, Some(selected))
    })
}

fn key_intent(model: &mut Model, key: KeyEvent, page: isize, home: Option<&Path>) -> Intent {
    if key.kind == KeyEventKind::Release {
        return Intent::None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c' | 'd'))
    {
        return Intent::Cancel;
    }
    if model.help {
        if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) {
            model.help = false;
        }
        return Intent::None;
    }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('r') => {
                model.toggle_deep();
                Intent::None
            }
            KeyCode::Char('l') if model.deep.is_some() => {
                model.refresh_deep();
                Intent::None
            }
            KeyCode::Char('f') => {
                model.toggle_files();
                Intent::None
            }
            KeyCode::Char('u') => {
                model.clear_query();
                Intent::None
            }
            KeyCode::Char('l') => Intent::Navigate(model.cwd.clone(), model.selected_path()),
            KeyCode::Char('g') => {
                home.map_or(Intent::None, |path| Intent::Navigate(path.to_owned(), None))
            }
            KeyCode::Char('n') => {
                model.move_selection(1);
                Intent::None
            }
            KeyCode::Char('p') => {
                model.move_selection(-1);
                Intent::None
            }
            _ => Intent::None,
        };
    }
    match key.code {
        KeyCode::Up => model.move_selection(-1),
        KeyCode::Down => model.move_selection(1),
        KeyCode::PageUp => model.move_selection(-page),
        KeyCode::PageDown => model.move_selection(page),
        KeyCode::Home => model.move_selection(isize::MIN),
        KeyCode::End => model.move_selection(isize::MAX),
        KeyCode::Esc | KeyCode::Left if model.deep.is_some() => model.leave_deep(),
        KeyCode::Backspace if model.deep.is_some() && model.query.is_empty() => model.leave_deep(),
        KeyCode::Left => return parent(model),
        KeyCode::Right | KeyCode::Tab => {
            return model
                .directory_target()
                .map_or(Intent::None, |path| Intent::Navigate(path, None));
        }
        KeyCode::Backspace if model.query.is_empty() => return parent(model),
        KeyCode::Backspace => model.pop_query(),
        KeyCode::Enter => return Intent::Finish,
        KeyCode::Esc if model.query.is_empty() => return Intent::Cancel,
        KeyCode::Esc => model.clear_query(),
        KeyCode::F(1) => model.help = true,
        KeyCode::Char('?') if model.query.is_empty() => model.help = true,
        KeyCode::Char('.') if model.query.is_empty() => model.toggle_hidden(),
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::ALT) => {
            model.push_query(&ch.to_string())
        }
        _ => {}
    }
    Intent::None
}

fn interrupted_flag() -> io::Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [
        signal_hook::consts::SIGTERM,
        signal_hook::consts::SIGINT,
        signal_hook::consts::SIGHUP,
        signal_hook::consts::SIGQUIT,
    ] {
        signal_hook::flag::register(signal, Arc::clone(&flag))?;
    }
    Ok(flag)
}

fn run(options: &Options) -> io::Result<Option<PathBuf>> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(io::Error::other(
            "interactive stdin and stderr are required; run ii in a terminal (stdout may be redirected)",
        ));
    }
    if env::var("TERM").as_deref() == Ok("dumb") {
        return Err(io::Error::other(
            "TERM=dumb does not support this interface",
        ));
    }
    let start = match &options.path {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => env::current_dir()?.join(path),
        None => env::current_dir()?,
    };
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let interrupted = interrupted_flag()?;
    let navigation = Scanner::new("ii-navigation")?;
    let previews = Scanner::new("ii-preview")?;
    let mut deep_search = Controller::default();
    let _session = Session::start()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stderr()))?;
    let mut model = Model::new(start, options.hidden);
    model.dirs_only = options.dirs_only;
    let mut navigation_id = navigation.request(model.cwd.clone());
    let mut preview_id = None;
    let mut preview_deadline: Option<Instant> = None;
    let mut finish = false;
    let mut dirty = true;
    let mut size = terminal.size()?;
    let theme = Theme::new(
        options.no_color || env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()),
    );

    loop {
        if interrupted.load(Ordering::Relaxed) {
            return Ok(None);
        }
        if let Some(result) = navigation.poll()
            && result.id == navigation_id
            && result.path == model.cwd
        {
            match result.result {
                Ok(listing) => {
                    model.cwd = result.resolved_path;
                    model.complete_navigation(listing);
                }
                Err(error) => {
                    finish = false;
                    model.fail_navigation(format!("{}: {}", result.path.display(), error));
                }
            }
            dirty = true;
        }
        // Only Enter can request completion. Tab and Right only navigate.
        // Keep Enter during a scan; a later key or scan failure cancels it.
        if !model.loading && std::mem::take(&mut finish) && model.listing.is_some() {
            return Ok(Some(model.cwd));
        }
        // Disabled and empty search do not create a worker or recurse.
        match deep_search.update(model.deep_query(), Instant::now()) {
            Ok(Some(progress)) => {
                model.apply_deep(progress);
                dirty = true;
            }
            Ok(None) => {}
            Err(error) => {
                model.apply_deep(Progress {
                    done: true,
                    error: Some(safe_label(&error.to_string())),
                    ..Progress::default()
                });
                dirty = true;
            }
        }
        let preview_enabled = !options.no_preview && size.width >= 90 && model.deep.is_none();
        let desired = if preview_enabled && !model.loading {
            model.selected_directory()
        } else {
            None
        };
        if desired != model.preview_path {
            previews.cancel();
            preview_id = None;
            model.preview = desired.as_ref().and_then(|path| model.cached(path));
            model.preview_path = desired;
            model.preview_error = None;
            model.preview_loading = model.preview_path.is_some();
            preview_deadline = model
                .preview_path
                .as_ref()
                .map(|_| Instant::now() + Duration::from_millis(45));
            dirty = true;
        }
        if preview_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            if let Some(path) = &model.preview_path {
                preview_id = Some(previews.request(path.clone()));
            }
            preview_deadline = None;
        }
        if let Some(result) = previews.poll()
            && Some(result.id) == preview_id
            && Some(&result.path) == model.preview_path.as_ref()
        {
            model.preview_loading = false;
            match result.result {
                Ok(listing) => {
                    model.cache_listing(result.path, Arc::clone(&listing));
                    model.preview = Some(listing);
                }
                Err(error) => {
                    model.preview = None;
                    model.preview_error = Some(safe_label(&error.to_string()));
                }
            }
            dirty = true;
        }
        if dirty {
            terminal.draw(|frame| ui::draw(frame, &mut model, theme, preview_enabled))?;
            dirty = false;
        }
        let wait = if model.loading
            || model.preview_loading
            || preview_deadline.is_some()
            || model.deep.as_ref().is_some_and(|deep| !deep.progress.done)
        {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(250)
        };
        if !event::poll(wait)? {
            continue;
        }
        let intent = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                finish = false;
                dirty = true;
                key_intent(
                    &mut model,
                    key,
                    ui::page_size(size.height) as isize,
                    home.as_deref(),
                )
            }
            Event::Paste(text) if !model.help => {
                finish = false;
                model.push_query(&text);
                dirty = true;
                Intent::None
            }
            Event::Resize(_, _) => {
                size = terminal.size()?;
                dirty = true;
                Intent::None
            }
            _ => Intent::None,
        };
        match intent {
            Intent::None => {}
            Intent::Cancel => return Ok(None),
            Intent::Finish => finish = true,
            Intent::Navigate(path, preferred) => {
                model.begin_navigation(path.clone(), preferred);
                navigation_id = navigation.request(path);
                dirty = true;
            }
        }
    }
}

fn write_path(path: &Path, print0: bool, mut output: impl Write) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        output.write_all(path.as_os_str().as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        let path = path
            .to_str()
            .ok_or_else(|| io::Error::other("selected path cannot be represented as UTF-8"))?;
        output.write_all(path.as_bytes())?;
    }
    if print0 {
        output.write_all(&[0])?;
    }
    output.flush()
}

fn main() -> std::process::ExitCode {
    match parse(env::args_os().skip(1)) {
        Ok(Command::Print(text)) => match io::stdout().write_all(text.as_bytes()) {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(_) => std::process::ExitCode::FAILURE,
        },
        Ok(Command::Run(options)) => match run(&options) {
            Ok(Some(path)) => match write_path(&path, options.print0, io::stdout().lock()) {
                Ok(()) => std::process::ExitCode::SUCCESS,
                Err(error) => {
                    eprintln!("ii: {error}");
                    std::process::ExitCode::FAILURE
                }
            },
            Ok(None) => std::process::ExitCode::from(130),
            Err(error) => {
                eprintln!("ii: {}", safe_label(&error.to_string()));
                std::process::ExitCode::FAILURE
            }
        },
        Err(error) => {
            eprintln!("ii: {error}\nTry ii --help");
            std::process::ExitCode::from(2)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ii::filesystem::{Entry, Listing};

    #[test]
    fn parsing_is_strict_but_accepts_dash_paths_after_separator() {
        assert!(parse(["--wat"].map(OsString::from)).is_err());
        assert!(parse(["a", "b"].map(OsString::from)).is_err());
        assert!(parse(["init", "nonsense"].map(OsString::from)).is_err());
        let Command::Run(options) =
            parse(["--hidden", "--dirs-only", "--", "-folder"].map(OsString::from))
                .unwrap_or_else(|_| panic!("parse failed"))
        else {
            panic!("not run")
        };
        assert!(options.hidden);
        assert!(options.dirs_only);
        assert_eq!(options.path.unwrap(), PathBuf::from("-folder"));
    }

    #[test]
    fn raw_path_protocol_preserves_trailing_newlines_and_metacharacters() {
        let mut output = Vec::new();
        write_path(Path::new("/tmp/a '$(echo nope)'\n\n"), false, &mut output).unwrap();
        assert_eq!(output, b"/tmp/a '$(echo nope)'\n\n");
        output.clear();
        write_path(Path::new("/tmp/a"), true, &mut output).unwrap();
        assert_eq!(output, b"/tmp/a\0");
    }

    #[test]
    fn q_filters_instead_of_quitting_and_escape_is_layered() {
        let mut model = Model::new(PathBuf::from("/"), false);
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(matches!(
            key_intent(&mut model, key(KeyCode::Char('q')), 10, None),
            Intent::None
        ));
        assert_eq!(model.query, "q");
        assert!(matches!(
            key_intent(&mut model, key(KeyCode::Esc), 10, None),
            Intent::None
        ));
        assert!(model.query.is_empty());
        assert!(matches!(
            key_intent(&mut model, key(KeyCode::Esc), 10, None),
            Intent::Cancel
        ));
    }

    #[test]
    fn right_never_navigates_into_a_file_and_enter_still_finishes_here() {
        let mut model = Model::new(PathBuf::from("/code"), false);
        model.complete_navigation(Arc::new(Listing {
            entries: vec![Entry::file(PathBuf::from("/code/main.rs"), false)],
            skipped: 0,
        }));
        let key = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(matches!(
            key_intent(&mut model, key(KeyCode::Right), 10, None),
            Intent::None
        ));
        assert!(model.message.as_deref().unwrap().contains("File selected"));
        assert!(matches!(
            key_intent(&mut model, key(KeyCode::Enter), 10, None),
            Intent::Finish
        ));
    }

    #[test]
    fn ctrl_f_toggles_files_but_an_ordinary_f_filters_and_help_is_modal() {
        let mut model = Model::new(PathBuf::from("/code"), false);
        key_intent(
            &mut model,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            10,
            None,
        );
        assert!(model.dirs_only);
        key_intent(
            &mut model,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE),
            10,
            None,
        );
        assert_eq!(model.query, "f");
        model.help = true;
        key_intent(
            &mut model,
            KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL),
            10,
            None,
        );
        assert!(model.dirs_only);
    }

    #[test]
    fn tab_and_right_navigate_without_finishing_even_with_cached_loading_rows() {
        for loading in [false, true] {
            for code in [KeyCode::Tab, KeyCode::Right] {
                let root = PathBuf::from("/code");
                let target = root.join("app");
                let mut model = Model::new(root, false);
                model.complete_navigation(Arc::new(Listing {
                    entries: vec![Entry::new(target.clone(), false)],
                    skipped: 0,
                }));
                model.loading = loading;
                let intent = key_intent(
                    &mut model,
                    KeyEvent::new(code, KeyModifiers::NONE),
                    10,
                    None,
                );
                assert!(matches!(intent, Intent::Navigate(path, None) if path == target));
            }
        }
    }

    #[test]
    fn tab_and_right_never_queue_completion_without_a_visible_selection() {
        for code in [KeyCode::Tab, KeyCode::Right] {
            let mut model = Model::new(PathBuf::from("/code"), false);
            assert!(model.loading);
            assert!(matches!(
                key_intent(
                    &mut model,
                    KeyEvent::new(code, KeyModifiers::NONE),
                    10,
                    None
                ),
                Intent::None
            ));
            // Empty loaded directories have exactly the same no-op behavior.
            model.complete_navigation(Arc::new(Listing::default()));
            assert!(matches!(
                key_intent(
                    &mut model,
                    KeyEvent::new(code, KeyModifiers::NONE),
                    10,
                    None
                ),
                Intent::None
            ));
        }
    }

    #[test]
    fn tab_and_right_keep_file_and_special_entry_guards() {
        use ii::filesystem::{EntryKind, FileKind};
        for kind in [
            EntryKind::File(FileKind::Code),
            EntryKind::UnresolvedLink,
            EntryKind::Special,
        ] {
            for code in [KeyCode::Tab, KeyCode::Right] {
                let root = PathBuf::from("/code");
                let mut model = Model::new(root.clone(), false);
                model.complete_navigation(Arc::new(Listing {
                    entries: vec![Entry::with_kind(root.join("entry"), false, kind)],
                    skipped: 0,
                }));
                assert!(matches!(
                    key_intent(
                        &mut model,
                        KeyEvent::new(code, KeyModifiers::NONE),
                        10,
                        None
                    ),
                    Intent::None
                ));
                assert!(model.message.is_some());
                assert_eq!(model.cwd, root);
            }
        }
    }

    #[test]
    fn only_enter_requests_completion_and_help_blocks_it() {
        for code in [
            KeyCode::Enter,
            KeyCode::Tab,
            KeyCode::Right,
            KeyCode::Left,
            KeyCode::Up,
            KeyCode::Down,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageUp,
            KeyCode::PageDown,
            KeyCode::Backspace,
            KeyCode::Esc,
            KeyCode::Char('q'),
            KeyCode::F(1),
        ] {
            let root = PathBuf::from("/code");
            let mut model = Model::new(root.clone(), false);
            model.complete_navigation(Arc::new(Listing {
                entries: vec![Entry::new(root.join("app"), false)],
                skipped: 0,
            }));
            let intent = key_intent(
                &mut model,
                KeyEvent::new(code, KeyModifiers::NONE),
                10,
                None,
            );
            assert_eq!(matches!(intent, Intent::Finish), code == KeyCode::Enter);
            model.help = true;
            assert!(matches!(
                key_intent(
                    &mut model,
                    KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                    10,
                    None
                ),
                Intent::None
            ));
        }
        let mut loading = Model::new(PathBuf::from("/code"), false);
        assert!(matches!(
            key_intent(
                &mut loading,
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                10,
                None
            ),
            Intent::Finish
        ));
    }

    #[test]
    fn ctrl_r_opens_search_escape_restores_browsing_enter_remains_only_finish() {
        let root = PathBuf::from("/code");
        let mut model = Model::new(root.clone(), false);
        model.complete_navigation(Arc::new(Listing {
            entries: vec![Entry::new(root.join("app"), false)],
            skipped: 0,
        }));
        let plain = |code| KeyEvent::new(code, KeyModifiers::NONE);
        assert!(matches!(
            key_intent(
                &mut model,
                KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
                10,
                None
            ),
            Intent::None
        ));
        assert!(model.deep.is_some());
        assert!(matches!(
            key_intent(&mut model, plain(KeyCode::Enter), 10, None),
            Intent::Finish
        ));
        assert!(matches!(
            key_intent(&mut model, plain(KeyCode::Esc), 10, None),
            Intent::None
        ));
        assert!(model.deep.is_none());
        assert_eq!(model.cwd, root);
    }
}
