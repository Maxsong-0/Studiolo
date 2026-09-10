#!/usr/bin/env python3
"""生成、预览、显式安装及检查Studiolo独立客户端资料目录。"""
import argparse
import json
import os
import sys

sys.dont_write_bytecode = True

from client_projection_fetch import fetch
from client_projection_io import read_bundle, write_bundle
from client_projection_render import render
from client_projection_schema import config, make_bundle
from client_projection_store import apply, check, plan
from replica_common import load_json, open_root, read_file, require


def absolute(value):
    require(os.path.isabs(value) and all(p not in ("", ".", "..") for p in value.split("/")[1:]))
    return value


def load_config(path):
    parent, name = os.path.split(absolute(path))
    fd = open_root(parent)
    try:
        raw = read_file(fd, name, 16384, private=False)[0]
    finally:
        os.close(fd)
    return config(load_json(raw))


def export(cfg):
    token = os.environ.get(cfg["token_env"], "")
    require(bool(token))
    snapshot = fetch(cfg, token)
    return make_bundle(cfg, snapshot, render(cfg, snapshot))


def run(args):
    if args.command == "generate":
        bundle = export(load_config(args.config))
        write_bundle(absolute(args.out), bundle)
        return {"status": "generated", "bundle_id": bundle["manifest"]["bundle_id"],
                "projection_key": bundle["manifest"]["projection_key"],
                "sources": len(bundle["manifest"]["sources"]), "out": args.out}, 0
    target = absolute(args.target)
    if args.command in ("plan", "apply"):
        bundle = read_bundle(absolute(args.bundle))
        result = (plan if args.command == "plan" else apply)(target, bundle)
        code = 3 if result["status"] == "activated_durability_unknown" else 2 if result["status"] == "conflict" else 0
        return result, code
    result = check(target)
    manifest = result.pop("manifest", None)
    result["source_status"] = "unknown_offline"
    if result["status"] != "ok":
        return result, 2
    result["generated_at_ms"] = manifest["generated_at_ms"]
    if args.online:
        try:
            remote = export(config(manifest["config"]))
        except Exception:
            result["source_status"] = "unavailable"
            return result, 3
        result["source_status"] = ("unchanged" if remote["manifest"]["projection_key"] ==
                                   manifest["projection_key"] else "source_changed")
        return result, 0 if result["source_status"] == "unchanged" else 2
    return result, 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    generate = commands.add_parser("generate", help="从已授权HTTP供给生成新私有目录")
    generate.add_argument("--config", required=True, help="配置文件绝对路径")
    generate.add_argument("--out", required=True, help="尚不存在的导出目录绝对路径")
    for command in ("plan", "apply"):
        sub = commands.add_parser(command, help="只读差异预览" if command == "plan" else "显式安装完整批次")
        sub.add_argument("--bundle", required=True)
        sub.add_argument("--target", required=True, help="独立客户端资料根目录的绝对路径")
    inspect = commands.add_parser("check", help="检查本地漂移，可另核对在线来源")
    inspect.add_argument("--target", required=True)
    inspect.add_argument("--online", action="store_true")
    args = parser.parse_args(argv)
    try:
        result, code = run(args)
    except Exception:
        # 不回显配置、路径中的异常内容、HTTP错误正文或秘密环境变量。
        result, code = {"status": "failed", "reason": "projection_validation_or_io_failed"}, 1
    print(json.dumps(result, ensure_ascii=False, sort_keys=True))
    return code


if __name__ == "__main__":
    sys.exit(main())
