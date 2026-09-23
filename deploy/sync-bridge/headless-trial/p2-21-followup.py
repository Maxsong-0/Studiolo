#!/usr/bin/env python3
"""T-132 P2 追加测试：排除项安全清除流程、下载中途强杀、SIGKILL 后锁恢复时间。

由用户执行：sudo -u studiolo-sync -H python3 -I - < ~/studiolo-p2/p2-21-followup.py
只动 ~/p2/mirror 与 ~/p2/observer；开跑前核对两个客户端都绑定 studiolo-p2-test，否则中止。
报告同时写到 ~/p2/matrix-<UTC时间>.txt。不读取、不打印任何 token 或密钥。
"""
import base64
import datetime
import hashlib
import json
import os
import pathlib
import pwd
import subprocess
import sys
import time

os.umask(0o077)
HOME = pathlib.Path.home()
OB = str(HOME / "bin/ob")
MIRROR = HOME / "p2/mirror"
OBSERVER = HOME / "p2/observer"
OBS_XDG = HOME / "p2/observer-xdg"
VAULT = "studiolo-p2-test"
TS = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
REPORT = HOME / f"p2/followup-{TS}.txt"
PNG_1X1 = base64.b64decode(
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg=="
)

lines = []
results = []


def log(s=""):
    print(s, flush=True)
    lines.append(s)


def result(sid, name, status, detail):
    results.append((sid, name, status, detail))
    log(f">> {sid} {name}: {status} — {detail}")


def base(client):
    return MIRROR if client == "mirror" else OBSERVER


def env_for(client):
    env = {"PATH": "/usr/bin:/bin", "HOME": str(HOME), "LANG": "C.UTF-8"}
    if client == "observer":
        env["XDG_CONFIG_HOME"] = str(OBS_XDG)
    return env


def ob(client, cmd, *extra, timeout=900):
    argv = [OB, cmd, "--path", str(base(client)), *extra]
    t0 = time.monotonic()
    p = subprocess.run(argv, env=env_for(client), stdin=subprocess.DEVNULL,
                       capture_output=True, text=True, timeout=timeout)
    return p.returncode, p.stdout, p.stderr, time.monotonic() - t0


def sync(client):
    rc, out, err, dt = ob(client, "sync")
    tail = [s for s in (out + err).strip().splitlines() if s.strip()][-2:]
    log(f"   [{client}] ob sync rc={rc} {dt:.1f}s | " + " / ".join(tail))
    return rc


def cfg(client, *changes):
    rc, out, err, _ = ob(client, "sync-config", *changes, "--json")
    if rc != 0:
        raise SystemExit(f"{client} sync-config 失败 rc={rc}: {err.strip()[:300]}")
    return json.loads(out)


def rel(client, r):
    return base(client) / r


def put(client, r, data):
    p = rel(client, r)
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_bytes(data)
    return p


def digest(client, r):
    p = rel(client, r)
    return hashlib.sha256(p.read_bytes()).hexdigest() if p.is_file() else None


def converge(client, check, tries=4, wait=3):
    for i in range(tries):
        sync(client)
        if check():
            return True
        if i + 1 < tries:
            time.sleep(wait)
    return False


def listing(client, folder):
    d = rel(client, folder)
    if not d.is_dir():
        return []
    return sorted(str(p.relative_to(base(client))) for p in d.rglob("*") if p.is_file())


def guard():
    m, o = cfg("mirror"), cfg("observer")
    for name, c, path in (("mirror", m, MIRROR), ("observer", o, OBSERVER)):
        shown = {k: c.get(k) for k in ("vaultName", "vaultPath", "syncMode", "conflictStrategy",
                                         "deviceName", "configDir", "fileTypes", "configs",
                                         "excludedFolders")}
        log(f"   {name} 配置：{json.dumps(shown, ensure_ascii=False)}")
        if c.get("vaultName") != VAULT or pathlib.Path(c.get("vaultPath", "")).resolve() != path.resolve():
            raise SystemExit(f"中止：{name} 未绑定 {VAULT} 或路径不符")
    if m.get("vaultId") != o.get("vaultId"):
        raise SystemExit("中止：两个客户端的 remote vaultId 不一致")


def main():
    log(f"# T-132 P2 追加测试 {TS}")
    guard()
    sync("mirror")
    sync("observer")

    log("## S7e 排除项的安全清除流程：排除期间先删镜像本地残留，再清排除项")
    r = "P2-Ex2/a.md"
    put("observer", r, b"v1\n")
    sync("observer")
    converge("mirror", lambda: digest("mirror", r) == hashlib.sha256(b"v1\n").hexdigest())
    cfg("mirror", "--excluded-folders", "P2-Ex2")
    sync("mirror")
    put("observer", r, b"v2\n")
    sync("observer")
    for f in listing("mirror", "P2-Ex2"):
        rel("mirror", f).unlink()
    rel("mirror", "P2-Ex2").rmdir()
    sync("mirror")
    sync("observer")
    sync("observer")
    still = rel("observer", r).read_bytes() if rel("observer", r).is_file() else None
    result("S7e-1", "排除期间删镜像本地残留不传播为远端删除", "PASS" if still == b"v2\n" else "FAIL",
           f"observer a.md={still!r}")
    cfg("mirror", "--excluded-folders", "")
    converge("mirror", lambda: digest("mirror", r) == hashlib.sha256(b"v2\n").hexdigest())
    sync("observer")
    m_v = rel("mirror", r).read_bytes() if rel("mirror", r).is_file() else None
    o_v = rel("observer", r).read_bytes() if rel("observer", r).is_file() else None
    result("S7e-2", "清排除项后镜像取回远端新版本、远端不被改写",
           "PASS" if (m_v == b"v2\n" and o_v == b"v2\n") else "FAIL", f"mirror={m_v!r} observer={o_v!r}")

    log("## S9d 下载中途 SIGKILL：最终路径上会不会出现半截文件")
    r9 = "S9d/中途.pdf"
    d9 = b"%PDF-1.4\n" + os.urandom(100 * 1024 * 1024) + b"\n%%EOF\n"
    put("observer", r9, d9)
    sync("observer")
    time.sleep(1)
    proc = subprocess.Popen([OB, "sync", "--path", str(MIRROR), "--continuous"], env=env_for("mirror"),
                            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    p = rel("mirror", r9)
    seen, t0 = None, time.monotonic()
    while time.monotonic() - t0 < 60:
        if p.is_file():
            n = p.stat().st_size
            if 0 < n < len(d9):
                seen = n
                break
            if n == len(d9):
                break
        others = [f for f in listing("mirror", "S9d") if f != r9]
        if others:
            seen = f"临时文件 {others}"
            break
        time.sleep(0.02)
    proc.kill()
    proc.wait()
    at_kill = p.stat().st_size if p.is_file() else None
    torn = p.is_file() and at_kill != len(d9)
    result("S9d-1", "下载中途最终路径是否可见半截内容", "OBSERVED",
           f"抓到中间态={seen}；杀掉时最终路径大小={at_kill}（完整 {len(d9)}）；最终路径半截={torn}；目录={listing('mirror', 'S9d')}")
    log("## S9e SIGKILL 后锁多久恢复")
    t1 = time.monotonic()
    rcs = []
    while time.monotonic() - t1 < 60:
        rc, out, err, _ = ob("mirror", "sync")
        rcs.append((round(time.monotonic() - t1, 1), rc))
        if rc == 0:
            break
        time.sleep(1)
    result("S9e", "SIGKILL 残留锁的恢复时间", "OBSERVED", f"尝试序列（秒, rc）={rcs}")
    want9 = hashlib.sha256(d9).hexdigest()
    converge("mirror", lambda: digest("mirror", r9) == want9)
    intact = digest("mirror", r9) == want9
    strays = [f for f in listing("mirror", "S9d") if f != r9]
    result("S9d-2", "重开后补齐且不留多余文件", "PASS" if (intact and not strays) else "FAIL",
           f"完整={intact}；多余文件={strays or '无'}")
    sync("observer")
    sync("observer")
    remote_ok = digest("observer", r9) == want9
    result("S9d-3", "半截文件没有被当成本地修改上传、远端未被损坏", "PASS" if remote_ok else "FAIL",
           f"observer 副本完整={remote_ok}；observer 目录={listing('observer', 'S9d')}")

    log("")
    log("## 汇总")
    for sid, name, status, _ in results:
        log(f"   {status:8} {sid} {name}")


if __name__ == "__main__":
    if pwd.getpwuid(os.getuid()).pw_name != "studiolo-sync":
        sys.exit("必须以 studiolo-sync 身份运行")
    try:
        main()
    finally:
        REPORT.write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"报告已写入 {REPORT}", flush=True)
