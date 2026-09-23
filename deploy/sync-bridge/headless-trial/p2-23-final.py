#!/usr/bin/env python3
"""T-132 P2 收尾：附件冲突是否生成冲突副本；清理测试数据（只删测试 Vault 里的内容）。

由用户执行：sudo -u studiolo-sync -H python3 -I - < ~/studiolo-p2/p2-23-final.py
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
REPORT = HOME / f"p2/final-{TS}.txt"
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


def log_lines(xdg_root, needle, n=25):
    out = []
    for f in sorted(pathlib.Path(xdg_root).glob("obsidian-headless/sync/*/sync.log")):
        for line in f.read_text(errors="replace").splitlines():
            if needle in line:
                out.append(line[:240])
    return out[-n:]


def main():
    log(f"# T-132 P2 收尾 {TS}")
    guard()
    sync("mirror")
    sync("observer")

    log("## S5b 附件（pdf）双方同时修改：冲突策略是否生成冲突副本")
    r = "S5b/附件.pdf"
    put("observer", r, b"%PDF-1.4\n" + os.urandom(2048) + b"\n%%EOF\n")
    sync("observer")
    converge("mirror", lambda: rel("mirror", r).is_file())
    m_ver = b"%PDF-1.4\nMIRROR\n" + os.urandom(1024) + b"\n%%EOF\n"
    o_ver = b"%PDF-1.4\nOBSERVER\n" + os.urandom(1024) + b"\n%%EOF\n"
    put("mirror", r, m_ver)
    put("observer", r, o_ver)
    sync("mirror")
    sync("observer")
    for _ in range(2):
        sync("mirror")
        sync("observer")
    found = {}
    for c in ("mirror", "observer"):
        for f in listing(c, "S5b"):
            b = rel(c, f).read_bytes()
            found[f"{c}:{f}"] = "MIRROR" if b == m_ver else ("OBSERVER" if b == o_ver else "其他")
    versions = set(found.values())
    copies = [k for k in found if "Conflicted" in k]
    result("S5b", "附件并发修改不丢任何一方", "PASS" if {"MIRROR", "OBSERVER"} <= versions else "FAIL",
           f"冲突副本={copies or '无'}；各处={json.dumps(found, ensure_ascii=False)}")
    for line in log_lines(HOME / ".config", "S5b"):
        log(f"   mirror sync.log | {line}")

    log("## 清理：由 observer 删除全部测试目录，经同步传到远端与镜像")
    folders = [d.name for d in OBSERVER.iterdir() if d.is_dir() and not d.name.startswith(".")]
    before = sum(p.stat().st_size for p in OBSERVER.rglob("*") if p.is_file())
    for name in folders:
        for p in sorted((OBSERVER / name).rglob("*"), reverse=True):
            p.unlink() if p.is_file() else p.rmdir()
        (OBSERVER / name).rmdir()
    sync("observer")
    converge("mirror", lambda: not [d for d in MIRROR.iterdir() if d.is_dir() and not d.name.startswith(".")])
    left_m = [d.name for d in MIRROR.iterdir() if not d.name.startswith(".")]
    left_o = [d.name for d in OBSERVER.iterdir() if not d.name.startswith(".")]
    result("CLEAN", "测试数据已从三处清除", "PASS" if not left_m and not left_o else "FAIL",
           f"删除前 observer 体积 {before / 1048576:.1f} MiB；删除目录={folders}；镜像残留={left_m or '无'}；observer 残留={left_o or '无'}")
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
