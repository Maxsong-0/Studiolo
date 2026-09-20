#!/usr/bin/env python3
"""Run non-root fictional human recovery contracts; never load production data."""
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent


def run(*args):
    subprocess.run(list(map(str, args)), check=True, cwd=ROOT, timeout=180)


def main():
    if sys.platform != "linux":
        raise SystemExit("Linux native validation required")
    run(sys.executable, "-I", "-B", ROOT / "deploy/agent/offline/run_snapshot_tests.py")
    run(sys.executable, "-I", "-B", ROOT / "deploy/human/offline/test_full_export.py")
    run(sys.executable, "-I", "-B", "-S", ROOT / "deploy/human/release/test_release.py")
    with tempfile.TemporaryDirectory(prefix="studiolo-factor-contract-") as name:
        binary = Path(name) / "factor-pipe"
        run("cc", "-Wall", "-Wextra", "-Werror", "-O2",
            ROOT / "deploy/human/offline/factor_pipe.c", "-o", binary)
        run(sys.executable, "-I", "-B", ROOT / "deploy/human/offline/test_portable_factor.py",
            "--filter-bin", binary)


if __name__ == "__main__":
    main()
