#!/usr/bin/python3
"""只解包已认证密文流到新root私有目录；不执行包内代码。"""
import ctypes
import json
import os
from pathlib import Path
import re
import resource
import stat
import sys
import tarfile
import unicodedata

BASE = Path('/var/lib/studiolo-dr-tests')

def require(ok):
    if not ok:
        raise ValueError('rejected')

def main():
    require(os.getuid() == os.geteuid() == 0 and sys.platform == 'linux')
    require(sys.flags.isolated and sys.flags.no_site and sys.dont_write_bytecode)
    require(len(sys.argv) == 2 and re.fullmatch('t094-[A-Za-z0-9-]{1,55}', sys.argv[1]))
    os.umask(0o077)
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    libc = ctypes.CDLL(None, use_errno=True)
    require(libc.prctl(4, 0, 0, 0, 0) == 0 and libc.prctl(3, 0, 0, 0, 0) == 0)
    for parent in [BASE, *BASE.parents]:
        item = parent.lstat()
        require(stat.S_ISDIR(item.st_mode) and item.st_uid == 0 and not item.st_mode & 0o7022)
    require(stat.S_IMODE(BASE.stat().st_mode) == 0o700)
    case = BASE / sys.argv[1]
    case.mkdir(mode=0o700)
    (case / 'INCOMPLETE').write_text('archive incoming\n')
    root = case / 'offline'
    root.mkdir(mode=0o700)
    names, total = set(), 0
    with tarfile.open(fileobj=sys.stdin.buffer, mode='r|') as archive:
        for member in archive:
            name = member.name
            if name in ('.', './'):
                require(member.isdir())
                continue
            if name.startswith('./'):
                name = name[2:]
            name = name.rstrip('/') if member.isdir() else name
            parts = name.split('/')
            gitkeep = parts[-1] == '.gitkeep' and name.startswith('human/release/example-vault/')
            require(len(parts) <= 72 and all(p and not p.startswith('.') for p in (parts[:-1] if gitkeep else parts)))
            require('\\' not in name and unicodedata.normalize('NFC', name) == name)
            require(not any(unicodedata.category(c).startswith('C') for c in name))
            require(len(name.encode()) <= 8192 and name not in names and len(names) < 100000)
            require(member.isfile() or member.isdir())
            require(not member.mode & 0o7022)
            names.add(name)
            destination = root / name
            require(destination.parent.is_dir())
            if member.isdir():
                destination.mkdir(mode=0o700)
                continue
            require(0 <= member.size <= 1024**3)
            total += member.size
            require(total <= 8 * 1024**3)
            mode = 0o700 if name == 'recovery/bin/studiolo-agent' else 0o600
            if name.startswith('human/release/bin/'):
                require(member.mode in (0o700, 0o755))
                mode = member.mode
            source = archive.extractfile(member)
            with destination.open('xb') as output:
                while chunk := source.read(1024 * 1024):
                    output.write(chunk)
                output.flush()
                os.fsync(output.fileno())
                os.fchmod(output.fileno(), mode)
            require(destination.stat().st_size == member.size)
    require((root / 'manifest.json').is_file() and (root / 'human/manifest.json').is_file())
    print(json.dumps({'archive_extracted': True, 'entries': len(names), 'bytes': total, 'case': str(case), 'complete': False}))

if __name__ == '__main__':
    try:
        main()
    except BaseException:
        os.write(2, b'restore_archive_failed\n')
        sys.exit(1)
