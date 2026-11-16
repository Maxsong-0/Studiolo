#!/usr/bin/env python3
"""三个生产单元的启动前对时闸门；不启停服务，不假定 chrony/timesyncd 的实现。"""
import json
from pathlib import Path
import subprocess
import sys
import time
import uuid

UNITS = {"studiolo-agent.service", "studiolo-human.service", "studiolo-tunnel.service"}


def wait_for_sync(read, clock, sleep, timeout=120):
    started = clock()
    while clock() - started < timeout:
        try:
            if read() == "yes":
                return round(clock() - started, 3)
        except (OSError, subprocess.SubprocessError):
            pass
        sleep(1)
    raise TimeoutError("clock_not_synchronized")


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in UNITS:
        return 2
    try:
        elapsed = wait_for_sync(lambda: subprocess.check_output(
            ["/usr/bin/timedatectl", "show", "--property=NTPSynchronized", "--value"],
            stderr=subprocess.DEVNULL, timeout=3, text=True).strip(), time.monotonic, time.sleep)
        boot = str(uuid.UUID(Path("/proc/sys/kernel/random/boot_id").read_text().strip()))
        print(json.dumps(dict(event="clock_ready", unit=sys.argv[1], boot_id=boot, waited_seconds=elapsed)), flush=True)
        return 0
    except Exception:
        print("clock_not_synchronized", file=sys.stderr, flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
