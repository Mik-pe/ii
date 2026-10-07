#!/usr/bin/env python3
"""Package an already-built release binary with documentation and SHA-256 checksums."""
import hashlib
from pathlib import Path
import subprocess
import tarfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main() -> None:
    metadata = tomllib.loads((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))
    version = metadata['package']['version']
    rustc = subprocess.run(['rustc', '-vV'], check=True, text=True, capture_output=True)
    target = next(line.removeprefix('host: ') for line in rustc.stdout.splitlines() if line.startswith('host: '))
    windows = 'windows' in target
    binary_name = 'ii.exe' if windows else 'ii'
    binary = ROOT / 'target' / 'release' / binary_name
    if not binary.is_file():
        raise SystemExit('Build the native release binary first: cargo build --locked --release')
    members = [(binary, binary_name)]
    for path in ['README.md', 'LICENSE', 'CONTRIBUTING.md', 'docs/ARCHITECTURE.md', 'docs/assets/cover.webp', 'shell/ii.sh', 'shell/ii.fish', 'shell/ii.ps1']:
        file = ROOT / path
        if not file.is_file():
            raise SystemExit(f'Missing package file: {path}')
        members.append((file, path))
    destination = ROOT / 'dist'
    destination.mkdir(exist_ok=True)
    name = f'ii-v{version}-{target}'
    archive = destination / (name + ('.zip' if windows else '.tar.gz'))
    if windows:
        with zipfile.ZipFile(archive, 'w', compression=zipfile.ZIP_DEFLATED) as package:
            for file, relative in members:
                package.write(file, f'{name}/{relative}')
    else:
        with tarfile.open(archive, 'w:gz') as package:
            for file, relative in members:
                package.add(file, arcname=f'{name}/{relative}', recursive=False)
    digest = hashlib.sha256(archive.read_bytes()).hexdigest()
    archive.with_name(archive.name + '.sha256').write_text(f'{digest}  {archive.name}\n', encoding='ascii')
    print(f'{archive.name}: {archive.stat().st_size} bytes\nSHA-256 {digest}')


if __name__ == '__main__':
    main()
