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
    def __init__(self, path, *options):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        self.transcript = bytearray()
        self.report = tempfile.TemporaryFile()
        try:
            self.process = subprocess.Popen(
                [sys.executable, '-c', SUPERVISOR, str(self.report.fileno()),
                 str(BINARY), '--no-preview', *options, str(path)],
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
        # Drain stderr while waiting so a full terminal buffer cannot deadlock.
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
        # Also reap a native child blocked in a failing test, not only its supervisor.
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

    def session(self, path=None, *options):
        session = Session(path or self.root, *options)
        self.sessions.append(session)
        session.expect(b'app' if path is None else b'FOLDERS')
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
        # The supervisor forwards TERM to ii, then observes its cleanup unchanged.
        session.process.send_signal(signal.SIGTERM)
        self.assertEqual(session.finish(), (130, b''))

    def test_print0_preserves_unusual_directory_names(self):
        path = self.root / "a 'quote' $dollar [bracket]\n\n"
        path.mkdir()
        session = self.session(path, '--print0')
        session.send(b'\r')
        self.assertEqual(session.finish(), (0, os.fsencode(str(path)) + b'\0'))


if __name__ == '__main__':
    unittest.main()
