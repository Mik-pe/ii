use std::env;
use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crossterm::cursor::{Hide, Show};
use crossterm::event::{self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::style::ResetColor;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::{Terminal, backend::CrosstermBackend};

use ii::filesystem::Scanner;
use ii::filter::safe_label;
use ii::model::Model;
use ii::ui::{self, Theme};

const HELP: &str = "ii — two taps, any directory\n\nUSAGE\n    ii [OPTIONS] [PATH]\n    ii init <bash|zsh|fish|powershell>\n\nOPTIONS\n    -a, --hidden       Show hidden directories\n        --no-preview   Use a single directory pane\n        --no-color     Use your terminal colors (also honors NO_COLOR)\n        --print0       Terminate the selected path with NUL (for scripts)\n    -h, --help         Print this help\n    -V, --version      Print the version\n        --             Treat the remaining argument as a path\n\nKEYS\n    ↑↓ select   → open   ← parent   Enter cd here   Tab cd selected\n    Type to filter. Esc clears the filter, then cancels. Ctrl-C always cancels.\n    . toggles hidden folders. Ctrl-L refreshes. ? opens help.\n\nSHELL SETUP (once in your shell profile)\n    bash:       eval \"$(ii init bash)\"\n    zsh:        eval \"$(ii init zsh)\"\n    fish:       ii init fish | source\n    PowerShell: ii init powershell | Out-String | Invoke-Expression\n\nThe UI uses stderr. A successful selection writes only the raw absolute path\n(with no trailing newline) to stdout. Cancellation exits 130 with no path.\nA subprocess cannot change its parent directory: install the shell function.\n";

#[derive(Default)]
struct Options {
    path: Option<PathBuf>,
    hidden: bool,
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
        let shell = arguments.next().ok_or("expected a shell: bash, zsh, fish, or powershell")?;
        if arguments.next().is_some() { return Err("too many arguments to init".into()); }
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
                Some("--") => { positional = true; continue; }
                Some("-h" | "--help") => return Ok(Command::Print(HELP)),
                Some("-V" | "--version") => return Ok(Command::Print(concat!("ii ", env!("CARGO_PKG_VERSION"), "\n"))),
                Some("-a" | "--hidden") => { options.hidden = true; continue; }
                Some("--no-preview") => { options.no_preview = true; continue; }
                Some("--no-color") => { options.no_color = true; continue; }
                Some("--print0") => { options.print0 = true; continue; }
                Some(flag) if flag.starts_with('-') => return Err(format!("unknown option: {}", safe_label(flag))),
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
        let _ = execute!(io::stderr(), DisableBracketedPaste, ResetColor, LeaveAlternateScreen, Show);
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
        execute!(io::stderr(), EnterAlternateScreen, EnableBracketedPaste, Hide)?;
        Ok(session)
    }
}

impl Drop for Session {
    fn drop(&mut self) { restore_terminal(); }
}

#[derive(Clone, Copy)]
enum Finish { Current, Selected }

enum Intent {
    None,
    Navigate(PathBuf, Option<PathBuf>),
    Finish(Finish),
    Cancel,
}

fn parent(model: &Model) -> Intent {
    model.parent().map_or(Intent::None, |(path, selected)| Intent::Navigate(path, Some(selected)))
}

fn key_intent(model: &mut Model, key: KeyEvent, page: isize, home: Option<&Path>) -> Intent {
    if key.kind == KeyEventKind::Release { return Intent::None; }
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c' | 'd') => Intent::Cancel,
            KeyCode::Char('u') => { model.clear_query(); Intent::None }
            KeyCode::Char('l') => Intent::Navigate(model.cwd.clone(), model.selected_path()),
            KeyCode::Char('g') => home.map_or(Intent::None, |path| Intent::Navigate(path.to_owned(), None)),
            KeyCode::Char('n') => { model.move_selection(1); Intent::None }
            KeyCode::Char('p') => { model.move_selection(-1); Intent::None }
            _ => Intent::None,
        };
    }
    if model.help {
        if matches!(key.code, KeyCode::Esc | KeyCode::F(1) | KeyCode::Char('?')) { model.help = false; }
        return Intent::None;
    }
    match key.code {
        KeyCode::Up => model.move_selection(-1),
        KeyCode::Down => model.move_selection(1),
        KeyCode::PageUp => model.move_selection(-page),
        KeyCode::PageDown => model.move_selection(page),
        KeyCode::Home => model.move_selection(isize::MIN),
        KeyCode::End => model.move_selection(isize::MAX),
        KeyCode::Left => return parent(model),
        KeyCode::Right => return model.selected_path().map_or(Intent::None, |path| Intent::Navigate(path, None)),
        KeyCode::Backspace if model.query.is_empty() => return parent(model),
        KeyCode::Backspace => model.pop_query(),
        KeyCode::Enter => return Intent::Finish(Finish::Current),
        KeyCode::Tab => return Intent::Finish(Finish::Selected),
        KeyCode::Esc if model.query.is_empty() => return Intent::Cancel,
        KeyCode::Esc => model.clear_query(),
        KeyCode::F(1) => model.help = true,
        KeyCode::Char('?') if model.query.is_empty() => model.help = true,
        KeyCode::Char('.') if model.query.is_empty() => model.toggle_hidden(),
        KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::ALT) => model.push_query(&ch.to_string()),
        _ => {}
    }
    Intent::None
}

fn interrupted_flag() -> io::Result<Arc<AtomicBool>> {
    let flag = Arc::new(AtomicBool::new(false));
    #[cfg(unix)]
    for signal in [signal_hook::consts::SIGTERM, signal_hook::consts::SIGINT, signal_hook::consts::SIGHUP, signal_hook::consts::SIGQUIT] {
        signal_hook::flag::register(signal, Arc::clone(&flag))?;
    }
    Ok(flag)
}

fn run(options: &Options) -> io::Result<Option<PathBuf>> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(io::Error::other("interactive stdin and stderr are required; run ii in a terminal (stdout may be redirected)"));
    }
    if env::var("TERM").as_deref() == Ok("dumb") {
        return Err(io::Error::other("TERM=dumb does not support this interface"));
    }
    let start = match &options.path {
        Some(path) if path.is_absolute() => path.clone(),
        Some(path) => env::current_dir()?.join(path),
        None => env::current_dir()?,
    };
    let home = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")).map(PathBuf::from);
    let interrupted = interrupted_flag()?;
    let navigation = Scanner::new("ii-navigation")?;
    let previews = Scanner::new("ii-preview")?;
    let _session = Session::start()?;
    let mut terminal = Terminal::new(CrosstermBackend::new(io::stderr()))?;
    let mut model = Model::new(start, options.hidden);
    let mut navigation_id = navigation.request(model.cwd.clone());
    let mut preview_id = None;
    let mut preview_deadline: Option<Instant> = None;
    let mut finish = None;
    let mut dirty = true;
    let mut size = terminal.size()?;
    let theme = Theme::new(options.no_color || env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty()));

    loop {
        if interrupted.load(Ordering::Relaxed) { return Ok(None); }
        if let Some(result) = navigation.poll() {
            if result.id == navigation_id && result.path == model.cwd {
                match result.result {
                    Ok(listing) => model.complete_navigation(listing),
                    Err(error) => {
                        finish = None;
                        model.fail_navigation(format!("{}: {}", result.path.display(), error));
                    }
                }
                dirty = true;
            }
        }
        // Remember Enter during a scan instead of making the user press it again.
        if !model.loading {
            if let Some(request) = finish.take() {
                match request {
                    Finish::Current if model.listing.is_some() => return Ok(Some(model.cwd)),
                    Finish::Selected => {
                        if let Some(path) = model.selected_path() {
                            model.begin_navigation(path.clone(), None);
                            navigation_id = navigation.request(path);
                            finish = Some(Finish::Current);
                            dirty = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        let preview_enabled = !options.no_preview && size.width >= 90;
        let desired = if preview_enabled && !model.loading { model.selected_path() } else { None };
        if desired != model.preview_path {
            previews.cancel();
            preview_id = None;
            model.preview = desired.as_ref().and_then(|path| model.cached(path));
            model.preview_path = desired;
            model.preview_error = None;
            model.preview_loading = model.preview_path.is_some();
            preview_deadline = model.preview_path.as_ref().map(|_| Instant::now() + Duration::from_millis(45));
            dirty = true;
        }
        if preview_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            if let Some(path) = &model.preview_path { preview_id = Some(previews.request(path.clone())); }
            preview_deadline = None;
        }
        if let Some(result) = previews.poll() {
            if Some(result.id) == preview_id && Some(&result.path) == model.preview_path.as_ref() {
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
        }
        if dirty {
            terminal.draw(|frame| ui::draw(frame, &mut model, theme, preview_enabled))?;
            dirty = false;
        }
        let wait = if model.loading || model.preview_loading || preview_deadline.is_some() {
            Duration::from_millis(8)
        } else {
            Duration::from_millis(250)
        };
        if !event::poll(wait)? { continue; }
        let intent = match event::read()? {
            Event::Key(key) if key.kind != KeyEventKind::Release => {
                finish = None;
                dirty = true;
                key_intent(&mut model, key, size.height.min(32).saturating_sub(9).max(1) as isize, home.as_deref())
            }
            Event::Paste(text) if !model.help => {
                finish = None;
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
            Intent::Finish(request) => finish = Some(request),
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
        let path = path.to_str().ok_or_else(|| io::Error::other("selected path cannot be represented as UTF-8"))?;
        output.write_all(path.as_bytes())?;
    }
    if print0 { output.write_all(&[0])?; }
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
                Err(error) => { eprintln!("ii: {error}"); std::process::ExitCode::FAILURE }
            },
            Ok(None) => std::process::ExitCode::from(130),
            Err(error) => { eprintln!("ii: {}", safe_label(&error.to_string())); std::process::ExitCode::FAILURE }
        },
        Err(error) => { eprintln!("ii: {error}\nTry ii --help"); std::process::ExitCode::from(2) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsing_is_strict_but_accepts_dash_paths_after_separator() {
        assert!(parse(["--wat"].map(OsString::from)).is_err());
        assert!(parse(["a", "b"].map(OsString::from)).is_err());
        assert!(parse(["init", "nonsense"].map(OsString::from)).is_err());
        let Command::Run(options) = parse(["--hidden", "--", "-folder"].map(OsString::from)).unwrap_or_else(|_| panic!("parse failed")) else { panic!("not run") };
        assert!(options.hidden);
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
        assert!(matches!(key_intent(&mut model, key(KeyCode::Char('q')), 10, None), Intent::None));
        assert_eq!(model.query, "q");
        assert!(matches!(key_intent(&mut model, key(KeyCode::Esc), 10, None), Intent::None));
        assert!(model.query.is_empty());
        assert!(matches!(key_intent(&mut model, key(KeyCode::Esc), 10, None), Intent::Cancel));
    }
}
