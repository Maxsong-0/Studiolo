#!/usr/bin/env python3
"""接收 stdin tar；用法 python3 replica_receiver.py --root /absolute/private/root。"""
import argparse
import json
import os
import signal
import sys
import tarfile
import uuid

from replica_archive import receive
from replica_common import DIR_FLAGS, Invalid, check_node
from replica_store import Store, remove_tree


def expired(_signal, _frame):
    raise TimeoutError("transfer_failed")


def main(argv=None):
    parser = argparse.ArgumentParser(description="接收完整、可验证的 Studiolo 白名单副本")
    parser.add_argument("--root", required=True)
    args = parser.parse_args(argv)
    os.umask(0o077)
    signal.signal(signal.SIGALRM, expired)
    signal.alarm(120)
    store = None
    stage_name = None
    stage = None
    try:
        store = Store(args.root)
        stage_name = ".stage-" + uuid.uuid4().hex
        os.mkdir(stage_name, 0o700, dir_fd=store.root)
        stage = os.open(stage_name, DIR_FLAGS, dir_fd=store.root)
        check_node(os.fstat(stage), True, private=True)
        sha, observed, expected = receive(sys.stdin.buffer, stage)
        receipt, reused = store.publish(stage_name, stage, sha, observed, expected)
        print(json.dumps({"status": "ready", "receipt": receipt, "reused": reused}, separators=(",", ":")))
        return 0
    except (OSError, ValueError, EOFError, TimeoutError, RecursionError, tarfile.TarError) as error:
        reason = "transfer_failed" if isinstance(error, (TimeoutError, EOFError, BrokenPipeError)) else "validation_failed"
        if store is not None:
            try:
                store.failed(reason)
            except (OSError, ValueError):
                pass
        print(json.dumps({"status": "failed", "last_error": reason}), file=sys.stderr)
        return 1
    finally:
        if stage is not None:
            os.close(stage)
        if store is not None:
            if stage_name is not None:
                try:
                    remove_tree(store.root, stage_name)
                except (FileNotFoundError, OSError, Invalid):
                    pass
            store.close()
        signal.alarm(0)


if __name__ == "__main__":
    sys.exit(main())
