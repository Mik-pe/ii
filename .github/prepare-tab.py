"""One-time, assertion-checked edit for the Tab navigation change.
Removed from the reviewed feature tree before merge.
"""
from pathlib import Path

changes = {}


def load(path):
    if path not in changes:
        changes[path] = Path(path).read_text(encoding='utf-8')
    return changes[path]


def replace(path, old, new, count=1):
    text = load(path)
    actual = text.count(old)
    assert actual == count, f'{path}: expected {count} occurrences of {old!r}, got {actual}'
    changes[path] = text.replace(old, new)


main = 'src/main.rs'
replace(main, 'Tab cd selected folder', 'Tab open folder')
replace(main, '#[derive(Clone, Copy)]\nenum Finish {\n    Current,\n    Selected,\n}\n', '')
replace(main, '    Finish(Finish),', '    Finish,')
replace(main, '        KeyCode::Right => {', '        KeyCode::Right | KeyCode::Tab => {')
replace(main, '        KeyCode::Tab => return Intent::Finish(Finish::Selected),\n', '')
replace(main, 'Intent::Finish(Finish::Current)', 'Intent::Finish', count=2)
replace(main, 'let mut finish = None;', 'let mut finish = false;')
replace(main, 'finish = None;', 'finish = false;', count=3)
text = load(main)
start = text.index('        // Remember Enter/Tab during a scan, but never allow a file as a cd target.')
end = text.index('        let preview_enabled = ', start)
changes[main] = text[:start] + '''        // Only Enter can request completion. Tab and Right only navigate.
        // Keep Enter during a scan; a later key or scan failure cancels it.
        if !model.loading && std::mem::take(&mut finish) && model.listing.is_some() {
            return Ok(Some(model.cwd));
        }
''' + text[end:]
replace(main, 'Intent::Finish(request) => finish = Some(request),', 'Intent::Finish => finish = true,')

rust_tests = '''

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
                    &mut model, KeyEvent::new(code, KeyModifiers::NONE), 10, None,
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
                key_intent(&mut model, KeyEvent::new(code, KeyModifiers::NONE), 10, None),
                Intent::None
            ));
            // Empty loaded directories have exactly the same no-op behavior.
            model.complete_navigation(Arc::new(Listing::default()));
            assert!(matches!(
                key_intent(&mut model, KeyEvent::new(code, KeyModifiers::NONE), 10, None),
                Intent::None
            ));
        }
    }

    #[test]
    fn tab_and_right_keep_file_and_special_entry_guards() {
        use ii::filesystem::{EntryKind, FileKind};
        for kind in [
            EntryKind::File(FileKind::Code), EntryKind::UnresolvedLink, EntryKind::Special,
        ] {
            for code in [KeyCode::Tab, KeyCode::Right] {
                let root = PathBuf::from("/code");
                let mut model = Model::new(root.clone(), false);
                model.complete_navigation(Arc::new(Listing {
                    entries: vec![Entry::with_kind(root.join("entry"), false, kind)],
                    skipped: 0,
                }));
                assert!(matches!(
                    key_intent(&mut model, KeyEvent::new(code, KeyModifiers::NONE), 10, None),
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
            KeyCode::Enter, KeyCode::Tab, KeyCode::Right, KeyCode::Left,
            KeyCode::Up, KeyCode::Down, KeyCode::Home, KeyCode::End,
            KeyCode::PageUp, KeyCode::PageDown, KeyCode::Backspace,
            KeyCode::Esc, KeyCode::Char('q'), KeyCode::F(1),
        ] {
            let root = PathBuf::from("/code");
            let mut model = Model::new(root.clone(), false);
            model.complete_navigation(Arc::new(Listing {
                entries: vec![Entry::new(root.join("app"), false)],
                skipped: 0,
            }));
            let intent = key_intent(&mut model, KeyEvent::new(code, KeyModifiers::NONE), 10, None);
            assert_eq!(matches!(intent, Intent::Finish), code == KeyCode::Enter);
            model.help = true;
            assert!(matches!(
                key_intent(&mut model, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10, None),
                Intent::None
            ));
        }
        let mut loading = Model::new(PathBuf::from("/code"), false);
        assert!(matches!(
            key_intent(&mut loading, KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE), 10, None),
            Intent::Finish
        ));
    }
'''
text = load(main).rstrip()
assert text.endswith('}')
changes[main] = text[:-1] + rust_tests + '}\n'

replace('src/model.rs', 'Right and Tab share this guard, including Tab remembered during loading.',
        'Right and Tab share this directory-only navigation guard; neither finishes.')
replace('src/ui.rs', '↑↓ select  → open  ← up  Enter cd here  Tab cd selected  Ctrl-F files  ? help',
        '↑↓ select  →/Tab open  ← up  Enter cd here  Ctrl-F files  ? help')
replace('src/ui.rs', '↑↓ select  → in  ← up  Enter cd  ? help',
        '↑↓ select  →/Tab in  ← up  Enter cd  ? help')
replace('src/ui.rs', 'Tab            Open selected folder and finish',
        'Tab            Open selected folder (stay in UI)')

pty = 'scripts/test_pty.py'
replace(pty, '    def drain(self):\n', '''    def assert_navigating(self):
        # Observe both liveness and the raw stdout protocol after a navigation key.
        # A ready stdout pipe means data or EOF: neither is allowed before Enter.
        deadline = time.monotonic() + 0.15
        while time.monotonic() < deadline:
            self.drain()
        assert self.process.poll() is None, 'navigation unexpectedly exited the UI'
        ready, _, _ = select.select([self.process.stdout], [], [], 0)
        assert not ready, 'navigation wrote a path or closed stdout before Enter'

    def drain(self):
''')
replace(pty, 'test_parent_remembers_selection_and_tab_finishes', 'test_parent_remembers_selection_and_tab_stays_in_ui')
replace(pty, "        session.send(b'\\t')\n        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'tools'))))", "        mark = session.send(b'\\t')\n        session.expect(b'lib', mark)\n        session.assert_navigating()\n        session.send(b'\\r')\n        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'tools'))))")
replace(pty, "        session.send(b'tl\\t')\n        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'tools'))))", "        mark = session.send(b'tl\\t')\n        session.expect(b'lib', mark)\n        session.assert_navigating()\n        session.send(b'\\r')\n        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'tools'))))")
replace(pty, "session.send(b'readme\\x15\\x06\\t')", "session.send(b'readme\\x15\\x06\\t\\r')")
replace(pty, "\n\nif __name__ == '__main__':", '''
    def test_tab_can_navigate_multiple_levels_then_cancel_without_cd(self):
        session = self.session()
        mark = session.send(b'\\t')
        session.expect(b'src', mark)
        session.assert_navigating()
        mark = session.send(b'\\t')
        session.expect(b'No visible entries', mark)
        session.assert_navigating()
        # A Tab in an empty directory must remain a no-op, not an implicit finish.
        session.send(b'\\t')
        session.assert_navigating()
        session.send(b'\\x1b')
        self.assertEqual(session.finish(), (130, b''))

    def test_tab_then_enter_in_one_burst_finishes_in_the_entered_directory(self):
        session = self.session()
        session.send(b'\\t\\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'app'))))

    def test_tab_then_right_can_continue_navigation_before_enter(self):
        session = self.session()
        mark = session.send(b'\\t')
        session.expect(b'src', mark)
        session.assert_navigating()
        session.send(b'\\x1b[B\\x1b[C\\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'app/tests'))))


if __name__ == '__main__':''')

replace('README.md', '| **Tab** | Open the selected **directory** and finish there. |',
        '| **Tab** | Open the selected **directory** inside the UI, just like **→**. |')
replace('README.md', '`Enter` always means **“take my shell here.”** `→` explores; `Tab` is the selected-folder shortcut. An Enter or Tab pressed while a directory loads is remembered. File guards apply to that delayed Tab too.',
        '**Enter is the only key that confirms a directory change in your shell.** Both `→` and `Tab` open the selected directory inside the UI and let you keep browsing. They never exit, and do nothing to files. After any number of navigation steps, `Esc` still cancels without changing the shell directory. An Enter pressed while a directory loads is remembered; navigation keys never queue a finish.')
replace('docs/ARCHITECTURE.md', 'Right enters a selected directory; Enter finishes at the header path. Tab combines opening a selected directory with finishing.',
        'Right and Tab enter a selected directory inside the UI without exiting. Only Enter finishes at the header path and returns it for the shell to cd into.')
replace('docs/ARCHITECTURE.md', 'Both Right and delayed Tab call `Model::directory_target`,',
        'Right and Tab share the same input branch and call `Model::directory_target`,')
replace('docs/ARCHITECTURE.md', 'Enter/Tab received during a scan is kept as a pending finish.',
        'Only Enter received during a scan is kept as a pending finish. Tab and Right act on the currently available selected directory; without an available selection they are no-ops and never schedule completion.')
replace('CONTRIBUTING.md', 'Preserve arrow semantics, the distinction between Enter and Tab, and selection when returning to a parent.',
        'Preserve arrow semantics and selection when returning to a parent. Tab and Right only enter a directory inside the UI; Enter is the only key that confirms cd in the shell. Test that Tab stays interactive, emits no stdout path, and can be followed by cancellation.')
replace('AGENTS.md', '- Enter finishes at the header path; Right and Tab accept directories only.',
        '- Enter is the only way to finish at the header path and cd in the shell. Right and Tab share directory-only navigation inside the UI; neither exits or queues completion.')

for path, text in changes.items():
    Path(path).write_text(text, encoding='utf-8', newline='\n')
    print(f'Updated {path}')
