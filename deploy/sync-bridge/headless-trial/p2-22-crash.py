#!/usr/bin/env python3
"""T-132 P2 第三轮：取 S9d 远端损坏的直接证据；测 mirror-remote／pull-only 两种模式下崩溃后的恢复。

由用户执行：sudo -u studiolo-sync -H python3 -I - < ~/studiolo-p2/p2-22-crash.py
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
REPORT = HOME / f"p2/crash-{TS}.txt"
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


def size_sha(client, r):
    p = rel(client, r)
    if not p.is_file():
        return None
    return p.stat().st_size, hashlib.sha256(p.read_bytes()).hexdigest()[:16]


def log_lines(xdg_root, needle, n=25):
    out = []
    for f in sorted(pathlib.Path(xdg_root).glob("obsidian-headless/sync/*/sync.log")):
        for line in f.read_text(errors="replace").splitlines():
            if needle in line:
                out.append(line[:240])
    return out[-n:]


def crash_case(sid, mode, folder):
    r = f"{folder}/崩溃.pdf"
    data = b"%PDF-1.4\n" + os.urandom(100 * 1024 * 1024) + b"\n%%EOF\n"
    want = hashlib.sha256(data).hexdigest()
    cfg("mirror", "--mode", mode)
    put("observer", r, data)
    sync("observer")
    time.sleep(1)
    proc = subprocess.Popen([OB, "sync", "--path", str(MIRROR), "--continuous"], env=env_for("mirror"),
                            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    p = rel("mirror", r)
    seen, t0 = None, time.monotonic()
    while time.monotonic() - t0 < 60:
        if p.is_file():
            n = p.stat().st_size
            if 0 < n < len(data):
                seen = n
                break
            if n == len(data):
                break
        time.sleep(0.02)
    proc.kill()
    proc.wait()
    at_kill = p.stat().st_size if p.is_file() else None
    t1 = time.monotonic()
    tries = []
    while time.monotonic() - t1 < 90:
        s = time.monotonic()
        rc, _, _, dt = ob("mirror", "sync")
        tries.append((round(s - t1, 1), rc, round(dt, 1)))
        if rc == 0:
            break
        time.sleep(1)
    for _ in range(3):
        sync("mirror")
    sync("observer")
    sync("observer")
    m_ok = digest("mirror", r) == want
    o_ok = digest("observer", r) == want
    result(f"{sid}-1", f"{mode}：强杀后镜像是否自愈", "PASS" if m_ok else "FAIL",
           f"抓到中间态={seen}；杀掉时大小={at_kill}（完整 {len(data)}）；镜像现为 {size_sha('mirror', r)}")
    result(f"{sid}-2", f"{mode}：远端未被半截文件覆盖", "PASS" if o_ok else "FAIL",
           f"observer 现为 {size_sha('observer', r)}；重试序列（开始秒, rc, 耗时秒）={tries}")
    for line in log_lines(HOME / ".config", "崩溃"):
        log(f"   mirror sync.log | {line}")


def main():
    log(f"# T-132 P2 第三轮 {TS}")
    guard()

    log("## D0 S9d 的直接证据")
    r9 = "S9d/中途.pdf"
    log(f"   mirror {r9} = {size_sha('mirror', r9)}")
    log(f"   observer {r9} = {size_sha('observer', r9)}")
    for line in log_lines(HOME / ".config", "中途"):
        log(f"   mirror sync.log | {line}")
    for line in log_lines(OBS_XDG, "中途"):
        log(f"   observer sync.log | {line}")

    log("## S9f mirror-remote 模式（只下载、回滚本地改动）")
    crash_case("S9f", "mirror-remote", "S9f")
    log("## S9g pull-only 模式（只下载、忽略本地改动）")
    crash_case("S9g", "pull-only", "S9g")

    cfg("mirror", "--mode", "bidirectional")
    log("   镜像模式已恢复为 bidirectional")
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
