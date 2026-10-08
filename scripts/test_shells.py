#!/usr/bin/env python3
"""Exercise the actual shell functions against a byte-exact fake ii executable."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]


@unittest.skipIf(os.name == 'nt', 'Unix shell integration tests')
class ShellIntegration(unittest.TestCase):
    def test_shell_protocol(self):
        for shell, source in [('bash', 'ii.sh'), ('zsh', 'ii.sh'), ('fish', 'ii.fish')]:
            executable = shutil.which(shell)
            if not executable:
                print(f'SKIP: {shell} is not installed')
                continue
            with self.subTest(shell=shell), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary).resolve()
                binary = root / 'ii'
                binary.write_text('#!/usr/bin/env python3\nimport os,sys\n'
                                  'code=int(os.environ.get("II_TEST_STATUS","0"))\n'
                                  'if sys.argv[1:2] == ["init"]:\n'
                                  ' sys.stdout.write("II_TEST_SETUP"); sys.exit(code)\n'
                                  'if code == 0:\n'
                                  ' sys.stdout.buffer.write(os.fsencode(os.environ["II_TEST_TARGET"]))\n'
                                  ' if "--print0" in sys.argv: sys.stdout.buffer.write(b"\\0")\n'
                                  'sys.exit(code)\n')
                binary.chmod(0o755)
                definitions = (ROOT / 'shell' / source).read_text()
                for name in ['normal', "a 'quote' $dollar [brackets]; literal", '-leading-dash', 'räv 日本', 'trailing\n\n', 'trailing...', 'init']:
                    target = root / name
                    target.mkdir()
                    env = dict(os.environ, PATH=f'{root}{os.pathsep}{os.environ["PATH"]}', II_TEST_TARGET=str(target), CDPATH=str(root))
                    calls = ['ii --hidden init', 'ii -- init'] if name == 'init' else ['ii']
                    for call in calls:
                        if shell == 'fish':
                            code = definitions + f'\n{call}; or exit $status\nprintf "%s\\0" "$PWD"\n'
                        else:
                            code = 'set -eu\n' + definitions + f'\n{call}\nprintf "%s\\0" "$PWD"\n'
                        result = subprocess.run([executable, '-c', code], cwd=root, env=env, capture_output=True, timeout=10)
                        self.assertEqual(result.returncode, 0, (shell, name, result.stderr))
                        self.assertEqual(result.stdout, os.fsencode(str(target)) + b'\0', (shell, name, call))
                for status in [130, 1, 2]:
                    env['II_TEST_STATUS'] = str(status)
                    if shell == 'fish':
                        code = definitions + '\nii\nset -l result $status\nprintf "%s\\0%s" "$PWD" "$result"\n'
                    else:
                        code = definitions + '\nii\nresult=$?\nprintf "%s\\0%s" "$PWD" "$result"\n'
                    result = subprocess.run([executable, '-c', code], cwd=root, env=env, capture_output=True, timeout=10)
                    self.assertEqual(result.stdout, os.fsencode(str(root)) + b'\0' + str(0 if status == 130 else status).encode(), (shell, status, result.stderr))
                # An informational request never changes the parent directory.
                env['II_TEST_STATUS'] = '0'
                result = subprocess.run([executable, '-c', definitions + '\nii init bash\n'], cwd=root, env=env, capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 0, (shell, result.stderr))
                self.assertEqual(result.stdout, b'II_TEST_SETUP', shell)
                if shell == 'fish':
                    code = definitions + '\nii --no-preview --help >/dev/null\nprintf "%s" "$PWD"\n'
                else:
                    code = definitions + '\nii --no-preview --help >/dev/null\nprintf "%s" "$PWD"\n'
                result = subprocess.run([executable, '-c', code], cwd=root, env=env, capture_output=True, timeout=10)
                self.assertEqual(result.stdout, os.fsencode(str(root)), shell)


if __name__ == '__main__':
    unittest.main()
