#!/usr/bin/env python3
"""Real terminal smoke tests: key input, raw mode, exit protocol, and signal cleanup."""
import fcntl
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


class Session:
    def __init__(self, path, *options):
        self.master, self.slave = pty.openpty()
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack('HHHH', 24, 100, 0, 0))
        self.before = termios.tcgetattr(self.slave)
        self.transcript = bytearray()

        def child_setup():
            os.setsid()
            fcntl.ioctl(0, termios.TIOCSCTTY, 0)

        self.process = subprocess.Popen([str(BINARY), '--no-preview', *options, str(path)], stdin=self.slave, stderr=self.slave, stdout=subprocess.PIPE,
                                        env=dict(os.environ, TERM='xterm-256color'), preexec_fn=child_setup)

    def send(self, data):
        mark = len(self.transcript)
        os.write(self.master, data)
        return mark

    def expect(self, needle, after=0):
        deadline = time.monotonic() + 10
        while needle not in self.transcript[after:]:
            if time.monotonic() > deadline:
                raise AssertionError(f'terminal did not show {needle!r}: {bytes(self.transcript)!r}')
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                self.transcript.extend(os.read(self.master, 65536))
            elif self.process.poll() is not None:
                raise AssertionError(f'ii exited early: {bytes(self.transcript)!r}')

    def finish(self):
        # Drain stderr's pty while waiting so a full terminal buffer cannot deadlock.
        deadline = time.monotonic() + 10
        while self.process.poll() is None:
            if time.monotonic() > deadline:
                raise AssertionError('ii did not exit')
            ready, _, _ = select.select([self.master], [], [], 0.05)
            if ready:
                self.transcript.extend(os.read(self.master, 65536))
        output = self.process.stdout.read()
        after = termios.tcgetattr(self.slave)
        assert self.before[3] == after[3], 'terminal local flags were not restored'
        return self.process.returncode, output

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait()
        self.process.stdout.close()
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
