#!/usr/bin/env python3
"""接收 stdin tar；用法 python3 replica_receiver.py --root /absolute/private/root --deadline 秒数。"""
import argparse
import json
import os
import signal
import sys
import tarfile
import uuid

from replica_archive import DEADLINE_BASE, MAX_DEADLINE, receive
from replica_common import DIR_FLAGS, Invalid, check_node, require
from replica_store import BaseMismatch, Store, remove_tree


def expired(_signal, _frame):
    raise TimeoutError("transfer_failed")


def apply(store, stage_name, stage, received):
    """整份照旧发布；确认只刷新回执；增量先按基线补齐未发来的文件，再与整份同样全量核对后发布。"""
    sha, observed, expected, base, provided = received
    if base is None:
        return store.publish(stage_name, stage, sha, observed, expected)
    prior, base_expected = store.base(base)
    try:
        if sha == base:
            require(not provided)
            return store.confirm(prior, sha, observed, expected), True
        require({p for p, entry in expected.items() if entry[0] == "dir"} <= provided)
        for path in sorted(set(expected) - provided):
            require(base_expected.get(path) == expected[path])
            store.copy(prior, stage, path, expected[path])
        return store.publish(stage_name, stage, sha, observed, expected)
    finally:
        os.close(prior)


def main(argv=None):
    parser = argparse.ArgumentParser(description="接收完整、可验证的 Studiolo 白名单副本")
    parser.add_argument("--root", required=True)
    parser.add_argument("--deadline", type=int, required=True)
    args = parser.parse_args(argv)
    os.umask(0o077)
    signal.signal(signal.SIGALRM, expired)
    store = None
    stage_name = None
    stage = None
    try:
        require(DEADLINE_BASE <= args.deadline <= MAX_DEADLINE)
        signal.alarm(args.deadline)
        store = Store(args.root)
        stage_name = ".stage-" + uuid.uuid4().hex
        os.mkdir(stage_name, 0o700, dir_fd=store.root)
        stage = os.open(stage_name, DIR_FLAGS, dir_fd=store.root)
        check_node(os.fstat(stage), True, private=True)
        receipt, reused = apply(store, stage_name, stage, receive(sys.stdin.buffer, stage))
        print(json.dumps({"status": "ready", "receipt": receipt, "reused": reused}, separators=(",", ":")))
        return 0
    except BaseMismatch as mismatch:
        # 基线对不上不是传输失败：现回执不动（现代次损坏时如实记账），请源端清基线、下一轮整份。
        if mismatch.corrupt:
            try:
                store.failed("validation_failed")
            except (OSError, ValueError):
                pass
        print(json.dumps({"status": "base_mismatch"}, separators=(",", ":")))
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
