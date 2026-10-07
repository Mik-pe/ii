#!/usr/bin/env python3
"""Real terminal tests, with a supervisor that checks cleanup before PTY hangup."""
import errno
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import unittest

BINARY = Path(sys.argv.pop(1) if len(sys.argv) > 1 else 'target/debug/ii').resolve()

# Keep the controlling session alive until after the native child exits. On
# macOS, querying the slave after its session leader exits returns ENOTTY.
# This supervisor observes cleanup; it must never restore the terminal itself.
SUPERVISOR = r'''
import fcntl
import json
import os
import signal
import subprocess
import sys
import termios

report_fd = int(sys.argv[1])
fcntl.ioctl(0, termios.TIOCSCTTY, 0)
before = repr(termios.tcgetattr(0))
child = subprocess.Popen(sys.argv[2:])

def forward(number, _frame):
    try:
        child.send_signal(number)
    except ProcessLookupError:
        pass

signal.signal(signal.SIGTERM, forward)
code = child.wait()
after = repr(termios.tcgetattr(0))
with os.fdopen(report_fd, 'w', encoding='utf-8') as report:
    json.dump({'before': before, 'after': after, 'status': code}, report)
sys.exit(code if code >= 0 else 128 - code)
'''


class Session:
    def __init__(self, path, *options, preview=False):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        self.transcript = bytearray()
        self.report = tempfile.TemporaryFile()
        try:
            self.process = subprocess.Popen(
                [sys.executable, '-c', SUPERVISOR, str(self.report.fileno()),
                 str(BINARY), *([] if preview else ['--no-preview']), *options, str(path)],
                stdin=self.slave, stderr=self.slave, stdout=subprocess.PIPE,
                pass_fds=(self.report.fileno(),), start_new_session=True,
                env=dict(os.environ, TERM='xterm-256color'))
        except BaseException:
            self.report.close()
            os.close(self.master)
            os.close(self.slave)
            raise

    def send(self, data):
        mark = len(self.transcript)
        os.write(self.master, data)
        return mark

    def drain(self):
        ready, _, _ = select.select([self.master], [], [], 0.05)
        if ready:
            try:
                self.transcript.extend(os.read(self.master, 65536))
            except OSError as error:
                if error.errno not in (errno.EIO, errno.ENXIO):
                    raise

    def expect(self, needle, after=0):
        deadline = time.monotonic() + 10
        while needle not in self.transcript[after:]:
            if time.monotonic() > deadline:
                raise AssertionError(f'terminal did not show {needle!r}: {bytes(self.transcript)!r}')
            self.drain()
            if needle not in self.transcript[after:] and self.process.poll() is not None:
                raise AssertionError(f'ii exited early: {bytes(self.transcript)!r}')

    def finish(self):
        deadline = time.monotonic() + 10
        while self.process.poll() is None:
            if time.monotonic() > deadline:
                raise AssertionError('ii did not exit')
            self.drain()
        self.drain()
        output = self.process.stdout.read()
        self.report.seek(0)
        result = json.load(self.report)
        assert result['before'] == result['after'], 'terminal attributes were not restored'
        assert result['status'] == self.process.returncode, 'supervisor/child status mismatch'
        return result['status'], output

    def close(self):
        if self.process.poll() is None:
            try:
                os.killpg(self.process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            self.process.wait()
        self.process.stdout.close()
        self.report.close()
        os.close(self.master)
        os.close(self.slave)


class TerminalIntegration(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        for directory in ['app/src', 'app/tests', 'notes', 'tools/lib']:
            (self.root / directory).mkdir(parents=True, exist_ok=True)
        self.sessions = []

    def tearDown(self):
        for session in self.sessions:
            session.close()
        self.temporary.cleanup()

    def session(self, path=None, *options, preview=False):
        session = Session(path or self.root, *options, preview=preview)
        self.sessions.append(session)
        session.expect(b'app' if path is None else (b'FOLDERS' if '--dirs-only' in options else b'CONTENTS'))
        return session

    def test_arrows_and_queued_enter(self):
        session = self.session()
        mark = session.send(b'\x1b[C')
        session.expect(b'src', mark)
        session.send(b'\x1b[B\x1b[C\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'app/tests'))))

    def test_parent_remembers_selection_and_tab_finishes(self):
        session = self.session()
        mark = session.send(b'\x1b[F\x1b[C')
        session.expect(b'lib', mark)
        mark = session.send(b'\x1b[D')
        session.expect(b'app', mark)
        session.send(b'\t')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'tools'))))

    def test_filter_then_tab(self):
        session = self.session()
        session.send(b'tl\t')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'tools'))))

    def test_cancel_never_prints_a_path(self):
        session = self.session()
        session.send(b'zzzz\x1b')
        time.sleep(0.1)
        self.assertIsNone(session.process.poll(), 'first Esc should only clear the filter')
        session.send(b'\x03')
        self.assertEqual(session.finish(), (130, b''))

    def test_external_term_restores_terminal(self):
        session = self.session()
        session.process.send_signal(signal.SIGTERM)
        self.assertEqual(session.finish(), (130, b''))

    def test_print0_preserves_unusual_directory_names(self):
        path = self.root / "a 'quote' $dollar [bracket]\n\n"
        path.mkdir()
        session = self.session(path, '--print0')
        session.send(b'\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(path)) + b'\0'))

    def test_files_are_visible_but_right_and_tab_never_return_a_file(self):
        (self.root / 'README.md').write_text('PRIVATE_FILE_CONTENT_NOT_FOR_THE_UI')
        session = self.session(preview=True)
        session.expect(b'README.md')
        session.send(b'readme\x1b[C\t')
        deadline = time.monotonic() + 0.2
        while time.monotonic() < deadline:
            session.drain()
        self.assertIsNone(session.process.poll(), 'Tab on a file must not exit')
        self.assertNotIn(b'PRIVATE_FILE_CONTENT_NOT_FOR_THE_UI', session.transcript)
        session.send(b'\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root))))

    def test_ctrl_f_restores_directory_navigation_after_selecting_a_file(self):
        (self.root / 'README.md').write_text('hello')
        session = self.session(None, '--dirs-only')
        self.assertNotIn(b'README.md', session.transcript)
        mark = session.send(b'\x06')
        session.expect(b'README.md', mark)
        # Filter selects the file; Ctrl-U clears the filter without changing it.
        # Hiding files must then select the first folder, not keep an invalid index.
        session.send(b'readme\x15\x06\t')
        self.assertEqual(session.finish(), (0, os.fsencode(str(self.root / 'app'))))

    def test_file_only_directory_can_be_selected_without_opening_any_file(self):
        path = self.root / 'notes'
        (path / 'readme.md').write_text('hello')
        session = self.session(path)
        session.expect(b'readme.md')
        session.send(b'\x1b[C\t\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(path))))

    def test_directory_preview_includes_files(self):
        (self.root / 'app' / 'Cargo.toml').write_text('[package]')
        session = self.session(preview=True)
        session.expect(b'Cargo.toml')
        session.send(b'\x03')
        self.assertEqual(session.finish(), (130, b''))

    def test_no_color_mode_keeps_file_names_without_rgb_escape_sequences(self):
        (self.root / 'main.rs').write_text('fn main() {}')
        session = self.session(None, '--no-color')
        session.expect(b'main.rs')
        self.assertNotIn(b'38;2;', session.transcript)
        self.assertNotIn(b'48;2;', session.transcript)
        session.send(b'\x03')
        self.assertEqual(session.finish(), (130, b''))


if __name__ == '__main__':
    unittest.main()
