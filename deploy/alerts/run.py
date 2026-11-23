#!/usr/bin/env python3
"""告警定时器入口；默认预演，只有 --deliver 才向外发心跳和告警。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import re
import secrets
import subprocess
import sys
import time

# -I 下仅加载安装包自己的固定模块，不使用当前目录、PYTHONPATH 或用户站点。
for name in ("residency", "bounded_process", "private_state", "heartbeat", "watch_state", "probe"):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(name + ".py"))
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)

from heartbeat import ping
from private_state import locked_state
from watch_state import CODES, delivered, failure_code, initial, observe
from residency import child_environment, harden
from bounded_process import output

REMOTE_COMMAND = "python3 -I -B -S /opt/studiolo-alert/current/deploy/alerts/run.py read-probe"


def remote_probe(host):
    if not re.fullmatch(r"[A-Za-z0-9._-]+@[A-Za-z0-9.-]+", host) or host.startswith("-"):
        raise ValueError("remote_invalid")
    reply = output(["ssh", "-oBatchMode=yes", "-oStrictHostKeyChecking=yes", "-oConnectTimeout=5", host, REMOTE_COMMAND], 45, child_environment())
    return json.loads(reply)


def tick(state, code, now, send):
    previous = state.load()
    start = initial(now)
    # 状态首次建立或被操作者重建时使用独立情节域，避免撞上旧审计而误判已投递。
    start["episode"] = secrets.randbits(62)
    updated, fire = observe(start if previous is None else previous, code, now)
    # 发送之前先记轮次；崩溃恢复不重用可能已经发送过的审计轮次。
    state.save(updated)
    if fire:
        if send(updated):
            state.save(delivered(updated, now))
        else:
            return False
    return True


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("probe", "read-probe", "heartbeat", "watch"))
    parser.add_argument("--node", choices=("n108", "n169"))
    parser.add_argument("--remote", default="maxsong@192.168.100.169")
    parser.add_argument("--state", default="/home/maxsong/.local/share/studiolo-alert/watch")
    parser.add_argument("--audit", default="/home/maxsong/.local/share/studiolo-alert/audit")
    parser.add_argument("--sender", default="/opt/studiolo-alert/current/bin/studiolo-alert")
    parser.add_argument("--deliver", action="store_true")
    args = parser.parse_args(argv)
    if args.mode == "read-probe":
        try:
            print(json.dumps(sys.modules["probe"].read_receipt(), separators=(",", ":")))
            return 0
        except Exception:
            print("probe_receipt_unavailable", file=sys.stderr)
            return 1
    if args.mode == "probe" or args.deliver:
        try:
            harden()
        except Exception:
            print("residency_unavailable", file=sys.stderr)
            return 1
    if args.mode == "probe":
        print(json.dumps(sys.modules["probe"].probe(), separators=(",", ":")))
        return 0
    if not args.deliver:
        print("dry_run_no_delivery")
        return 0
    if (args.mode == "watch" and args.node != "n108") or (args.mode == "heartbeat" and args.node != "n169"):
        parser.error("watch 使用 n108，heartbeat 使用 n169")
    if args.mode == "heartbeat":
        try:
            sys.modules["probe"].save_receipt(sys.modules["probe"].probe())
        except Exception:
            print("probe_receipt_unavailable", file=sys.stderr)
            return 1
        heartbeats = ping(args.node)
        print(json.dumps({"heartbeat_hc": heartbeats[0], "heartbeat_worker": heartbeats[1]}))
        return 0 if all(heartbeats) else 1
    try:
        code = failure_code(remote_probe(args.remote))
    except Exception:
        code = "A01"

    def send(value):
        command = [args.sender, args.audit, CODES[value["code"]], value["code"], str(value["episode"]), str(value["round"]), "--deliver"]
        try:
            reply = subprocess.run(command, stdout=subprocess.PIPE, timeout=25, check=False)
            return reply.returncode == 0 and reply.stdout.strip() in (b"accepted", b"already_accepted")
        except Exception:
            return False
    try:
        with locked_state(args.state) as state:
            accepted = tick(state, code, int(time.time()), send)
        # 看门狗自己的状态、审计或发送坏了时停发它的心跳，由外部死人开关兜住。
        heartbeats = ping(args.node) if accepted else [False, False]
        print(json.dumps({"heartbeat_hc": heartbeats[0], "heartbeat_worker": heartbeats[1]}))
        print(json.dumps({"probe_code": code, "dispatch_ok": accepted}))
        return 0 if accepted and all(heartbeats) else 1
    except Exception:
        print("watch_state_unavailable", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
