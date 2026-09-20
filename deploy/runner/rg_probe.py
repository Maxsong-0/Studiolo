#!/usr/bin/python3
"""Fixed live sandbox probe; exit 0 only when every local containment claim holds.
Gateway fence, budget and cancellation probes are separate deployment tests.
"""
import json
import os
import pathlib
import socket
import subprocess
import sys


def check(condition):
    if not condition:
        raise SystemExit(41)


def main():
    check(os.getuid() == 65534 and os.getgid() == 65534)
    check(os.readlink('/proc/self/ns/net') != sys.argv[1])
    status = pathlib.Path('/proc/self/status').read_text()
    check('CapEff:\t0000000000000000' in status)
    for name in ('/root', '/Users', '/var/run/docker.sock', '/run/docker.sock',
                 '/home/runner/.ssh', '/home/runner/.aws', '/home/runner/.codex/auth.json',
                 '/home/runner/.claude.json', '/ops/policy.yaml', '/ops/approvals'):
        check(not os.path.exists(name))
    for name in ('/escape-probe', '/inputs/escape-probe', '/usr/escape-probe'):
        try:
            with open(name, 'wb') as output:
                output.write(b'escape')
        except OSError:
            pass
        else:
            raise SystemExit(42)
    for address in ('1.1.1.1', '169.254.169.254', '192.168.100.1'):
        client = socket.socket()
        client.settimeout(0.2)
        try:
            client.connect((address, 443))
        except OSError:
            pass
        else:
            raise SystemExit(43)
        finally:
            client.close()
    # Read the actual cgroup ceilings, not requested systemd properties.
    cgroup = pathlib.Path('/proc/self/cgroup').read_text().strip().split('::')[-1]
    croot = pathlib.Path('/sys/fs/cgroup') / cgroup.lstrip('/')
    for filename, maximum in (('pids.max', 64), ('memory.max', 1073741824)):
        value = (croot / filename).read_text().strip()
        check(value.isdecimal() and 0 < int(value) <= maximum)
    cpu = (croot / 'cpu.max').read_text().split()
    check(cpu[0] != 'max' and int(cpu[0]) <= 2 * int(cpu[1]))
    disk = os.statvfs('/work')
    check(disk.f_blocks * disk.f_frsize <= 536870912)
    check(pathlib.Path('/run/studiolo-gateway.sock').is_socket())
    if len(sys.argv) == 3:
        provider = sys.argv[2]
        check(provider in ('claude', 'codex'))
        args = [provider, '--help'] if provider == 'claude' else [provider, 'exec', '--help']
        result = subprocess.run(args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                stderr=subprocess.DEVNULL, timeout=10, check=False)
        check(result.returncode == 0 and len(result.stdout) <= 1024 * 1024)
        required = (('--bare', '--input-format', '--disallowedTools', '--strict-mcp-config')
                    if provider == 'claude' else
                    ('--ephemeral', '--ignore-user-config', '--ignore-rules', '--disable', '--json'))
        check(all(flag.encode() in result.stdout for flag in required))
        version = subprocess.run([provider, '--version'], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.DEVNULL,
                                 timeout=5, check=False)
        check(version.returncode == 0 and 0 < len(version.stdout) <= 1024)
        value = version.stdout.decode('utf-8').strip()
        check(value and len(value) <= 256 and not any(ord(c) < 32 for c in value))
        print(json.dumps({'binary_version': value, 'protocol_capabilities': list(required)}))


if __name__ == '__main__':
    main()
