#!/usr/bin/env python3
"""用虚构 Vault 串联 Python 捕获/接收与真实 MCP 二进制；不连接真实服务器。"""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import socket
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from replica_common import ALLOW  # noqa: E402
import tempfile
import time
import urllib.error
import urllib.request

sys.dont_write_bytecode = True
from replica_capture import archive, capture

SCRIPTS = Path(__file__).resolve().parent
INDEX = "项目/Studiolo/00-总览/INDEX.md"
SPEC = importlib.util.spec_from_file_location("agent_check", SCRIPTS / "check-agent.py")
CHECK = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(CHECK)


def require(ok, message):
    if not ok:
        raise RuntimeError(message)


def write(path, body):
    path.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
    path.write_text(body, encoding="utf-8")
    path.chmod(0o600)


class Pipeline:
    def __init__(self, binary, root):
        self.binary, self.root = binary, root
        self.source, self.remote, self.data = (root / name for name in ("source", "remote", "data"))
        self.proc = None
        write(self.source / INDEX, "# Studiolo · 总览\n\nFIRST_CONTENT\n")
        write(self.source / "项目/Studiolo/remove.md", "# 待删除\n")
        write(self.source / "_规则/rules.md", "# 规则\n")
        # 捕获端对 ALLOW 里每个目录都 open_dir，缺一个即整批失败；从 ALLOW 派生避免漏建。
        for allow in ALLOW:
            (self.source / allow).mkdir(mode=0o700, parents=True, exist_ok=True)
        (self.source / "项目/Studiolo/empty").mkdir(mode=0o700)
        write(self.source / "School/private.md", "DO_NOT_SEND\n")
        write(self.source / "项目/Studiolo/.hidden", "DO_NOT_SEND\n")
        (self.data / "audit").mkdir(mode=0o700, parents=True)
        self.data.chmod(0o700)
        self.admin("init")
        self.token = self.issue_type_token("fixture")
        self.id = 0
        self.session = None

    def admin(self, sub, *args, data=True):
        # 策略库子命令（policy-init、policy-status）只接受 --policy：调用方传 data=False，不附 --data。
        extra = ("--data", str(self.data)) if data else ()
        result = subprocess.run([self.binary, "admin", sub, *extra, *args],
                                capture_output=True, text=True, timeout=20)
        # 带上子命令名：这条消息为所有 admin 子命令共用，只写「初始化失败」会把
        # 后续子命令的失败误报成 init 的失败（2026-09-17 实际误导过一条排查线）。
        require(result.returncode == 0, f"隔离身份库 admin {sub} 失败")
        return result.stdout

    def admin_tty(self, sub, *args):
        """在真实 PTY 里跑 admin 子命令（附 --data），返回 (退出码, 终端输出)。

        `type-issue`（docs/contracts/agent-supply.md §2）与 `policy-bootstrap`（crates/agent-server/src/admin/policy.rs）
        只在真实前台控制终端运行：无控制终端、或任一标准流被重定向，都会被拒绝且什么都不做。
        本验收脚本因此给它一个真实 PTY，**不是**给产品开非 TTY 后门——那等于把门禁本身废掉。
        """
        pid, master = pty.fork()
        if pid == 0:                       # 子进程：三个标准流都是这个 PTY
            try:
                os.execve(self.binary, [self.binary, "admin", sub, "--data", str(self.data), *args],
                          {"PATH": os.environ.get("PATH", "/usr/bin:/bin"), "LANG": "C.UTF-8"})
            finally:
                # execve 成功不返回。失败时子进程**绝不能**掉回父进程的 Python 栈：
                # 它会沿 run() 的 `with tempfile.TemporaryDirectory()` 一路 unwind，
                # 在父进程仍在使用时把夹具目录整个删掉，而父进程不抛任何异常、
                # 毫不知情地继续跑（2026-09-18 同构复现实测）。
                os._exit(127)
        seen = bytearray()
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if not select.select([master], [], [], 0.5)[0]:
                continue
            try:
                chunk = os.read(master, 4096)
            except OSError:
                break
            if not chunk:
                break
            seen.extend(chunk)
        os.close(master)
        code = os.waitstatus_to_exitcode(self.reap(pid))
        output = bytes(seen).decode("utf-8", "replace")
        # 已持有的类型 token 不得出现在任何 admin 子命令的终端输出里；__init__ 签发之前尚无该属性，不检。
        held = getattr(self, "token", None)
        require(held is None or held not in output, f"admin {sub} 的终端输出含类型 token")
        return code, output

    def issue_type_token(self, kind):
        """在真实 PTY 里签发类型 token：它是 MCP 供给面的全部身份，产品只把它写真实前台控制终端。"""
        code, output = self.admin_tty("type-issue", "--type", kind)
        require(code == 0, "类型 token 签发失败（真实 PTY 下）")
        found = re.search(r"sag1_[0-9a-f]{64}", output)
        require(found is not None, "控制终端上没有出现类型 token")
        return found.group(0)

    @staticmethod
    def reap(pid, limit=10):
        """带期限地收走子进程，未按期退出者判为失败。

        关掉主设备端通常会让子进程收到 SIGHUP 而退出，但不能把验收押在这上面：
        裸 `waitpid(pid, 0)` 在子进程挂住时会无限阻塞，管线会**卡死而不是失败**。
        """
        deadline = time.monotonic() + limit
        while time.monotonic() < deadline:
            done, status = os.waitpid(pid, os.WNOHANG)
            if done == pid:
                return status
            time.sleep(0.05)
        os.kill(pid, signal.SIGKILL)
        return os.waitpid(pid, 0)[1]       # 非零，由调用方判失败

    def push(self, truncated=False):
        with archive(capture(str(self.source))) as stream:
            data = stream.read()
        result = subprocess.run([sys.executable, "-B", str(SCRIPTS / "replica_receiver.py"), "--root", str(self.remote),
                                 "--deadline", "120"],
                                input=data[:700] if truncated else data,
                                capture_output=True, timeout=20)
        require(result.returncode != 0 if truncated else result.returncode == 0,
                "截断传输未拒绝" if truncated else "捕获与接收协议不兼容")
        return json.loads((self.remote / "control/state.json").read_text())

    def start(self, policy_dir=None):
        # 端口占位与子进程同寿（T-171）：带 SO_REUSEADDR 只绑定不监听，agent 经 tokio 绑定同一端口；
        # 占位期间同机别的进程的自动分配选不到它（Linux 默认 ip_autobind_reuse=0）。旧写法取号后立即释放，
        # 窗口内端口可能落到别人手里，「连得上」就会连到别人的服务。共享绑定依赖 Linux 语义（本脚本只在 CI 跑）。
        self.reservation = socket.socket()
        self.reservation.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.reservation.bind(("127.0.0.1", 0))
        port = self.reservation.getsockname()[1]
        self.url = f"http://127.0.0.1:{port}/mcp"
        self.log = tempfile.TemporaryFile()
        # 缺省不接策略平面（M3 兼容模式），参数与改前逐字相同；给目录即追加 --policy。
        policy = () if policy_dir is None else ("--policy", str(policy_dir))
        self.proc = subprocess.Popen([self.binary, "serve", "--replica-root", str(self.remote),
                                      "--replica-max-age-secs", "300",
                                      *[arg for allow in ALLOW for arg in ("--allow", allow)], "--data", str(self.data),
                                      *policy, "--port", str(port)],
                                     stdout=self.log, stderr=self.log)
        # 就绪只认 agent 自己的启动行（绑定成功后才打印），不认「能连上」。
        announcement = f"studiolo-agent：{self.url}".encode()
        deadline = time.monotonic() + 20
        while announcement not in self.output():
            require(self.proc.poll() is None, "隔离 MCP 服务启动失败")
            require(time.monotonic() < deadline, "等待隔离 MCP 服务超时")
            time.sleep(0.05)
        try:
            urllib.request.urlopen(self.url, timeout=5).close()
        except urllib.error.HTTPError as error:
            require(error.code == 401, "匿名访问未被拒绝")
            error.close()
        else:
            raise RuntimeError("匿名访问未被拒绝")
        self.session = None
        value = self.rpc("initialize", {"protocolVersion": CHECK.PROTOCOL, "capabilities": {},
                                       "clientInfo": {"name": "replica-pipeline-test", "version": "1"}})
        text = value["result"]["instructions"]
        self.rpc("notifications/initialized", {}, notification=True)
        return text

    def output(self):
        """读已写入的 stdout／stderr；用 pread 不动子进程与本进程共享的文件偏移，免得它后续写入覆盖开头。"""
        fd = self.log.fileno()
        return os.pread(fd, os.fstat(fd).st_size, 0)

    def stop(self):
        if self.proc:
            self.proc.terminate()
            try:
                self.proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait(timeout=5)
            self.proc = None
            self.log.close()
            # 子进程已收走，端口占位随之交还。
            self.reservation.close()

    def rpc(self, method, params, notification=False):
        self.id += 1
        body = {"jsonrpc": "2.0", "method": method, "params": params}
        if not notification:
            body["id"] = self.id
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                   "Authorization": "Bearer " + self.token, "MCP-Protocol-Version": CHECK.PROTOCOL}
        if self.session:
            headers["Mcp-Session-Id"] = self.session
        request = urllib.request.Request(self.url, data=json.dumps(body).encode(), headers=headers, method="POST")
        with urllib.request.urlopen(request, timeout=20) as response:
            if not self.session:
                self.session = response.headers.get("Mcp-Session-Id")
            if notification:
                require(response.status == 202, "会话通知未接受")
                return None
            return CHECK.payload(response.read().decode(), response.headers.get("Content-Type", ""), self.id)

    def call(self, name, args=None, denied=False):
        value = self.rpc("tools/call", {"name": name, "arguments": args or {}})
        if denied:
            require("error" in value or value.get("result", {}).get("isError") is True,
                    "完整性失败后仍返回成功内容")
            require("SECOND_CONTENT" not in json.dumps(value), "完整性失败泄露正文")
            return value
        require("error" not in value and value["result"].get("isError") is not True, "MCP 读取失败")
        return "".join(item["text"] for item in value["result"]["content"])

    def four(self, receipt, content):
        texts = [self.call("handshake"), self.call("read_note", {"path": INDEX}),
                 self.call("list_entries", {"project": "项目/Studiolo"}),
                 self.call("search", {"project": "项目/Studiolo", "text": content})]
        for text in texts:
            require(receipt["generation"] in text and receipt["source_digest"] in text,
                    "四工具未绑定同一完整批次")
            require("DO_NOT_SEND" not in text, "传出了白名单外或隐藏内容")
        require(content in texts[1] and content in texts[3], "正文与搜索未更新")
        return texts


def deployed_probes(pipeline):
    """对同一个真实二进制跑一遍上线脚本在 4401 阶段实例上用的那组探针
    （deploy/agent/upgrade-read-supply.sh.in：check-agent 三种凭据、副本回执、HTTP 文本外壳）。
    探针单测只面对假服务器；不在这里与真实二进制相遇，工具清单一变，上线当天才会发现。"""
    token_file = pipeline.root / "probe-token"
    write(token_file, pipeline.token + "\n")
    origin = pipeline.url[:-len("/mcp")]
    for script, extra in (("check-agent.py", []), ("check-agent.py", ["--no-token"]),
                          ("check-agent.py", ["--bad-token"]),
                          ("check-agent-replica.py", ["--expected", "ready"]),
                          ("check-agent-http.py", [])):
        target = origin if script == "check-agent-http.py" else pipeline.url
        result = subprocess.run([sys.executable, "-B", str(SCRIPTS / script), target, str(token_file), *extra],
                                capture_output=True, text=True, timeout=120)
        # 探针的 stderr 只含它自己预定义的诊断，不含 token、会话 id 或正文，可以原样带出。
        detail = (result.stderr.strip().splitlines() or ["无诊断输出"])[-1]
        require(result.returncode == 0, f"上线探针 {script} {' '.join(extra)}对真实二进制失败：{detail}")
    token_file.unlink()


def run(binary):
    # 夹具的中间目录按默认权限创建；umask 002 下它们是 0775，会被捕获端按设计拒绝。与 CI 同一 umask。
    os.umask(0o022)
    with tempfile.TemporaryDirectory(prefix="studiolo-replica-pipeline-") as directory:
        root = Path(directory).resolve()
        pipeline = Pipeline(binary, root)
        try:
            first = pipeline.push()
            instructions = pipeline.start()
            require("副本状态 ready" in instructions, "初始握手未显示可信新鲜度")
            first_texts = pipeline.four(first["receipt"], "FIRST_CONTENT")
            require("remove.md" in first_texts[2] and "empty" in first_texts[2], "目录同步不完整")
            print("通过：Python 捕获/接收 → Rust 初始化 → 四工具同批读取及匿名拒绝")
            deployed_probes(pipeline)
            print("通过：上线探针（check-agent 三种凭据、副本回执、HTTP 外壳）对真实二进制全过")

            write(pipeline.source / INDEX, "# Studiolo · 总览\n\nSECOND_CONTENT\n")
            (pipeline.source / "项目/Studiolo/remove.md").unlink()
            write(pipeline.source / "项目/Studiolo/new.md", "# 新增\n")
            second = pipeline.push()
            second_texts = pipeline.four(second["receipt"], "SECOND_CONTENT")
            require("remove.md" not in second_texts[2] and "new.md" in second_texts[2], "新增/删除未生效")
            require(first["receipt"]["generation"] != second["receipt"]["generation"], "内容更新未产生新代次")
            print("通过：内容更新、文件增删与空目录；不等待后台轮询即可读到新批次")

            failed = pipeline.push(truncated=True)
            require(failed["status"] == "failed" and failed["receipt"] == second["receipt"], "失败覆盖了成功回执")
            require("sync_failed" in pipeline.call("read_note", {"path": INDEX}), "失败后未标记旧副本")
            recovered = pipeline.push()
            require("副本状态 ready" in pipeline.call("handshake"), "成功同步后未恢复")
            print("通过：半传输拒绝、保留完整旧批次、失败提示和重试恢复")

            # 仅修改隔离夹具时间并重启，验证启动时不会把旧副本扫描时间当作来源确认。
            pipeline.stop()
            old = int(time.time() * 1000) - 301_000
            recovered["last_attempt_at_ms"] = old
            recovered["receipt"]["source_observed_at_ms"] = old
            recovered["receipt"]["received_at_ms"] = old
            write(pipeline.remote / "control/state.json", json.dumps(recovered))
            require("副本状态 stale" in pipeline.start(), "重启把旧副本误标为新鲜")
            require("副本状态 stale" in pipeline.call("read_note", {"path": INDEX}), "正文缺少陈旧标记")
            print("通过：服务重启保留来源时间，陈旧状态覆盖握手与正文")

            live = pipeline.remote / "releases" / recovered["receipt"]["generation"] / "vault" / INDEX
            before = live.read_bytes()
            live.write_bytes(before.replace(b"SECOND_CONTENT", b"TAMPER_CONTENT"))
            for name, args in (("handshake", {}), ("read_note", {"path": INDEX}),
                               ("list_entries", {"project": "项目/Studiolo"}),
                               ("search", {"project": "项目/Studiolo", "text": "CONTENT"})):
                pipeline.call(name, args, denied=True)
            print("通过：同大小字节漂移立即阻断四工具供给")
            repaired = pipeline.push()
            require(repaired["receipt"]["generation"] != recovered["receipt"]["generation"],
                    "副本漂移后复用了损坏代次")
            pipeline.four(repaired["receipt"], "SECOND_CONTENT")
            print("通过：源内容未变时重新同步仍可修复损坏副本")
        finally:
            pipeline.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", help="已构建 studiolo-agent 二进制路径")
    args = parser.parse_args()
    try:
        run(str(Path(args.binary).resolve(strict=True)))
    except Exception as error:
        # 夹具状态也只报告预定义错误，不打印 token 或完整 HTTP 正文。
        message = str(error) if isinstance(error, RuntimeError) else type(error).__name__
        print(f"副本管线验收失败：{message}", file=sys.stderr)
        sys.exit(1)
