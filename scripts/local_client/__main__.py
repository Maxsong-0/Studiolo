"""Explicit startup and per-session MCP configuration, without global client edits."""
import argparse
import json
import os
from pathlib import Path
import sys
import subprocess

from .config import load, new_token, private_create, private_read


def main():
    parser = argparse.ArgumentParser(description="Studiolo 本机 Codex / Claude Code 连接程序")
    parser.add_argument("--config", required=True, type=Path, help="本机私有配置绝对路径")
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("serve")
    sub.add_parser("pair-code", help="只在自己的终端显示本机配对码")
    sub.add_parser("check")
    mcp = sub.add_parser("mcp")
    mcp.add_argument("--client", required=True)
    mcp.add_argument("--channel", action="store_true")
    settings = sub.add_parser("client-config", help="显示每个客户端需要的 MCP 设置，不含令牌")
    settings.add_argument("--client", required=True)
    launch = sub.add_parser("launch", help="用本次 MCP 设置打开已登记的原生客户端；不改全局配置")
    launch.add_argument("--client", required=True)
    init = sub.add_parser("init", help="创建一个本机客户端登记；不会改个人客户端设置")
    add = sub.add_parser("add-client", help="离线向现有配置追加一个客户端；需先停止 serve")
    for target in (init, add):
        target.add_argument("--client", required=True)
        target.add_argument("--paid-usage-disabled", action="store_true", help="本机用户确认账号额外付费已关闭/无余额；不是平台强制开关")
        target.add_argument("--remote")
        target.add_argument("--thread")
        target.add_argument("--studiolo-token-file", type=Path, required=True)
        target.add_argument("--endpoint", required=True)
        target.add_argument("--cwd", type=Path, required=True)
        target.add_argument("--binary", type=Path, required=True)
        target.add_argument("--provider", choices=["codex", "claude"], required=True)
    init.add_argument("--origin", required=True)
    init.add_argument("--port", type=int, default=4318)
    args = parser.parse_args()
    os.umask(0o077)
    if not args.config.is_absolute():
        raise ValueError("配置路径必须为绝对路径")
    if args.command in {"init", "add-client"}:
        from .config import identifier
        identifier(args.client)
        previous = None
        lock_fd = None
        if args.command == "add-client":
            import fcntl
            checked = load(args.config)
            if any(c["id"] == args.client for c in checked["clients"]):
                raise ValueError("客户端标识已登记")
            lock_fd = os.open(checked["state_directory"] / "service.lock", os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW, 0o600)
            try:
                fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            except OSError:
                os.close(lock_fd)
                raise ValueError("请先停止本机连接程序，再追加登记") from None
            previous = json.loads(private_read(args.config))
        elif args.config.exists():
            raise ValueError("配置已存在；使用 add-client 追加登记")
        private_read(args.studiolo_token_file)
        args.config.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        root = Path(previous["state_directory"]) if previous else args.config.parent / "state"
        if not previous:
            root.mkdir(mode=0o700)
        pair, bridge = root / "pairing.token", root / (args.client + ".bridge.token")
        if not previous:
            new_token(pair)
        new_token(bridge)
        c = {"id": args.client, "provider": args.provider,
            "mode": "codex-queue" if args.provider == "codex" else "claude-channel",
            "binary": str(args.binary), "cwd": str(args.cwd), "project": "项目/Studiolo",
            "mcp_endpoint": args.endpoint, "mcp_token_file": str(args.studiolo_token_file),
            "bridge_token_file": str(bridge), "included_quota_only": True,
            "paid_usage_disabled": args.paid_usage_disabled}
        if args.thread:
            c["thread"] = args.thread
        if args.remote:
            c["remote"] = args.remote
        cfg = previous or {"version": 1, "port": getattr(args, "port", 4318), "origin": getattr(args, "origin", None),
            "state_directory": str(root), "pairing_token_file": str(pair), "clients": [c]}
        if previous:
            cfg["clients"].append(c)
            temporary = args.config.with_name(args.config.name + ".new")
            private_create(temporary, json.dumps(cfg, ensure_ascii=False, indent=2) + "\n")
            try:
                load(temporary)
                os.replace(temporary, args.config)
            finally:
                temporary.unlink(missing_ok=True)
                os.close(lock_fd)
        else:
            private_create(args.config, json.dumps(cfg, ensure_ascii=False, indent=2) + "\n")
            load(args.config)
        print("已创建本机登记；请配置该会话 MCP 后启动 serve。")
        return
    cfg = load(args.config)
    if args.command == "serve":
        from .server import serve
        serve(cfg)
    elif args.command == "mcp":
        from .stdio import run
        run(cfg, args.client, args.channel)
    elif args.command == "pair-code":
        print(cfg["pairing_token"])
    elif args.command == "check":
        from .native import subscription_status
        print(json.dumps({"clients": [{"id": c["id"], **subscription_status(c)} for c in cfg["clients"]]}, ensure_ascii=False))
    elif args.command in {"client-config", "launch"}:
        c = next(c for c in cfg["clients"] if c["id"] == args.client)
        entry = str(Path(__file__).resolve().parents[1] / "studiolo-local.py")
        argv = [entry, "--config", str(args.config), "mcp", "--client", c["id"]]
        if c["provider"] == "claude":
            argv.append("--channel")
        result = {"command": sys.executable, "args": argv}
        if args.command == "client-config":
            if c["provider"] == "codex":
                print("[mcp_servers.studiolo]\ncommand = " + json.dumps(sys.executable) + "\nargs = " + json.dumps(argv, ensure_ascii=False))
            else:
                print(json.dumps({"mcpServers": {"studiolo": result}}, ensure_ascii=False, indent=2))
                print("Claude 启动时还需 --dangerously-load-development-channels server:studiolo（官方开发 Channel 预览；保留原生确认）", file=sys.stderr)
        else:
            from .native import clean_environment, subscription_status
            if not subscription_status(c)["ready"]:
                raise ValueError("订阅登录或仅订阅用量声明尚未确认")
            if c["provider"] == "codex":
                command = [c["binary"], "-C", c["cwd"], "-c", 'forced_login_method="chatgpt"',
                    "-c", "mcp_servers.studiolo.command=" + json.dumps(sys.executable),
                    "-c", "mcp_servers.studiolo.args=" + json.dumps(argv), "resume", c["thread"]]
                if c.get("remote"):
                    command += ["--remote", c["remote"]]
            else:
                command = [c["binary"], "--mcp-config", json.dumps({"mcpServers": {"studiolo": result}}),
                    "--dangerously-load-development-channels", "server:studiolo"]
                if c.get("thread"):
                    command += ["--resume", c["thread"]]
            raise SystemExit(subprocess.call(command, cwd=c["cwd"], env=clean_environment()))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError):
        print("本机配置或连接不可用；请核对登记与私有文件权限。", file=sys.stderr)
        raise SystemExit(1)
