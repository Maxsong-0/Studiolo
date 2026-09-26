#!/usr/bin/env python3
"""T-132 P2 契约矩阵：两台 headless 客户端（mirror／observer）在测试 Vault 上往返。

由用户执行：sudo -u studiolo-sync -H python3 -I - < ~/studiolo-p2/p2-20-matrix.py
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
import signal
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
REPORT = HOME / f"p2/matrix-{TS}.txt"
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
    log(f"# T-132 P2 契约矩阵 {TS}")
    log("## C0 身份与配置核对")
    guard()
    types = "image,audio,video,pdf,unsupported"
    m = cfg("mirror", "--mode", "bidirectional", "--conflict-strategy", "conflict",
            "--file-types", types, "--configs", "", "--excluded-folders", "")
    o = cfg("observer", "--mode", "bidirectional", "--conflict-strategy", "conflict",
            "--file-types", types, "--configs", "app,appearance", "--excluded-folders", "")
    ok = (m.get("conflictStrategy") == "conflict" and m.get("configs") == []
          and set(types.split(",")) <= set(m.get("fileTypes", [])))
    result("C0", "镜像配置（冲突文件／全附件类型／关闭配置同步）", "PASS" if ok else "FAIL",
           f"mirror={json.dumps({k: m.get(k) for k in ('syncMode', 'conflictStrategy', 'fileTypes', 'configs')}, ensure_ascii=False)}；"
           f"observer={json.dumps({k: o.get(k) for k in ('syncMode', 'conflictStrategy', 'fileTypes', 'configs')}, ensure_ascii=False)}")
    sync("mirror")
    sync("observer")

    log("## S1 observer→mirror：中文路径与各类附件")
    s1 = {
        "S1-中文 目录/笔记 一.md": "# 测试\n中文正文\n".encode(),
        "S1-中文 目录/图.png": PNG_1X1,
        "S1-中文 目录/文档.pdf": b"%PDF-1.4\n" + os.urandom(4096) + b"\n%%EOF\n",
        "S1-中文 目录/音.mp3": b"ID3" + os.urandom(4096),
        "S1-中文 目录/包.zip": b"PK\x05\x06" + b"\0" * 18,
        "S1-中文 目录/纯文本.txt": "txt 内容\n".encode(),
        "S1-中文 目录/数据.json": b'{"a": 1}\n',
    }
    for r, d in s1.items():
        put("observer", r, d)
    sync("observer")
    same = lambda: all(digest("mirror", r) == hashlib.sha256(d).hexdigest() for r, d in s1.items())
    converge("mirror", same)
    missing = [r for r, d in s1.items() if digest("mirror", r) != hashlib.sha256(d).hexdigest()]
    result("S1", "中文路径＋png/pdf/mp3/zip/txt/json 下行", "PASS" if not missing else "FAIL",
           "全部逐字节一致" if not missing else f"未到或不一致：{missing}")

    log("## S2 mirror→observer：反向上行")
    s2 = {"S2-反向/来自镜像.md": "镜像写入\n".encode(),
          "S2-反向/附件.pdf": b"%PDF-1.4\n" + os.urandom(8192) + b"\n%%EOF\n"}
    for r, d in s2.items():
        put("mirror", r, d)
    sync("mirror")
    same2 = lambda: all(digest("observer", r) == hashlib.sha256(d).hexdigest() for r, d in s2.items())
    converge("observer", same2)
    result("S2", "镜像上行到另一设备", "PASS" if same2() else "FAIL",
           "逐字节一致" if same2() else f"observer 现有：{listing('observer', 'S2-反向')}")

    log("## S3 A→B→A")
    r3 = "S3/aba.md"
    for v in (b"A\n", b"B\n", b"A\n"):
        put("observer", r3, v)
        sync("observer")
    converge("mirror", lambda: digest("mirror", r3) == hashlib.sha256(b"A\n").hexdigest())
    got = rel("mirror", r3).read_bytes() if rel("mirror", r3).is_file() else None
    result("S3", "A→B→A 终态", "PASS" if got == b"A\n" else "FAIL", f"mirror 终态={got!r}")

    log("## S4 下载写入方式（ADR-005 试跑项 1：原地截断还是替换）")
    p4 = rel("mirror", r3)
    before = p4.stat() if p4.is_file() else None
    put("observer", r3, b"C-" + os.urandom(64).hex().encode() + b"\n")
    want4 = digest("observer", r3)
    sync("observer")
    converge("mirror", lambda: digest("mirror", r3) == want4)
    after = p4.stat() if p4.is_file() else None
    if before and after:
        how = "inode 不变＝原地写入" if before.st_ino == after.st_ino else "inode 变化＝替换写入"
        result("S4", "镜像下载更新的写入方式", "OBSERVED",
               f"{how}；inode {before.st_ino}→{after.st_ino}；mode {oct(after.st_mode & 0o777)}；nlink {after.st_nlink}")
    else:
        result("S4", "镜像下载更新的写入方式", "FAIL", "文件缺失，无法比较")

    log("## S5 冲突策略＝生成冲突文件")
    r5 = "S5/conflict.md"
    put("observer", r5, "基线\n".encode())
    sync("observer")
    converge("mirror", lambda: digest("mirror", r5) == hashlib.sha256("基线\n".encode()).hexdigest())
    put("mirror", r5, "MIRROR 版本\n".encode())
    put("observer", r5, "OBSERVER 版本\n".encode())
    sync("mirror")
    sync("observer")
    for _ in range(2):
        sync("mirror")
        sync("observer")
    texts = {}
    for c in ("mirror", "observer"):
        for f in listing(c, "S5"):
            texts[f"{c}:{f}"] = rel(c, f).read_text(errors="replace").strip()
    both = ("MIRROR 版本" in texts.values()) and ("OBSERVER 版本" in texts.values())
    extra = [k for k in texts if not k.endswith(":S5/conflict.md")]
    result("S5", "并发修改不丢任何一方", "PASS" if both else "FAIL",
           f"冲突副本={extra}；各处内容={json.dumps(texts, ensure_ascii=False)}")

    log("## S6 删除传播（Bridge 侧另有熔断，这里只观察 headless 行为）")
    r6 = "S1-中文 目录/纯文本.txt"
    rel("observer", r6).unlink()
    sync("observer")
    converge("mirror", lambda: not rel("mirror", r6).exists())
    result("S6", "另一设备删除后镜像是否跟删", "OBSERVED",
           "镜像已删除" if not rel("mirror", r6).exists() else "镜像仍保留")

    log("## S7 排除文件夹（D5：改排除项不得删远端）")
    ex = {"P2-Excluded/a.md": b"v1\n", "P2-Excluded/b.pdf": b"%PDF-1.4\n" + os.urandom(2048) + b"\n%%EOF\n"}
    for r, d in ex.items():
        put("observer", r, d)
    sync("observer")
    converge("mirror", lambda: all(rel("mirror", r).is_file() for r in ex))
    cfg("mirror", "--excluded-folders", "P2-Excluded")
    sync("mirror")
    kept_local = listing("mirror", "P2-Excluded")
    sync("observer")
    sync("observer")
    remote_ok = all(digest("observer", r) == hashlib.sha256(d).hexdigest() for r, d in ex.items())
    result("S7a", "加排除项后远端文件仍在", "PASS" if remote_ok else "FAIL",
           f"observer 现有={listing('observer', 'P2-Excluded')}；镜像本地留存={kept_local}")
    put("observer", "P2-Excluded/a.md", b"v2\n")
    sync("observer")
    sync("mirror")
    sync("mirror")
    m_a = rel("mirror", "P2-Excluded/a.md")
    got_a = m_a.read_bytes() if m_a.is_file() else None
    result("S7b", "排除后镜像不再下载该目录变更", "PASS" if got_a != b"v2\n" else "FAIL",
           f"镜像 a.md={got_a!r}")
    put("mirror", "P2-Excluded/from-mirror.md", "镜像在排除目录里新建\n".encode())
    sync("mirror")
    sync("observer")
    sync("observer")
    leaked = rel("observer", "P2-Excluded/from-mirror.md").exists()
    result("S7c", "排除后镜像不上传该目录", "PASS" if not leaked else "FAIL",
           "observer 未收到" if not leaked else "observer 收到了 from-mirror.md")
    cfg("mirror", "--excluded-folders", "")
    for _ in range(2):
        sync("mirror")
        sync("observer")
    o_a = rel("observer", "P2-Excluded/a.md")
    o_val = o_a.read_bytes() if o_a.is_file() else None
    m_val = m_a.read_bytes() if m_a.is_file() else None
    b_ok = rel("observer", "P2-Excluded/b.pdf").is_file()
    result("S7d", "清除排除项后：远端新版本不被镜像旧副本覆盖、不删远端",
           "PASS" if (o_val == b"v2\n" and b_ok) else "FAIL",
           f"observer a.md={o_val!r}；mirror a.md={m_val!r}；b.pdf 在远端={b_ok}；目录={listing('observer', 'P2-Excluded')}")

    log("## S8 镜像关闭配置同步")
    put("observer", ".obsidian/appearance.json", b'{"theme": "obsidian", "p2": true}\n')
    sync("observer")
    sync("mirror")
    sync("mirror")
    m8 = rel("mirror", ".obsidian/appearance.json")
    leaked8 = m8.is_file() and b'"p2": true' in m8.read_bytes()
    result("S8", "另一设备的配置文件不进镜像", "PASS" if not leaked8 else "FAIL",
           "镜像未收到 appearance.json" if not leaked8 else "镜像收到了 appearance.json")

    log("## S9 持续模式：常驻内存、子进程、SIGTERM 与中途终止")
    s9 = {f"S9/批量-{i:02d}.md": f"第 {i} 份\n".encode() for i in range(20)}
    s9["S9/大附件.pdf"] = b"%PDF-1.4\n" + os.urandom(20 * 1024 * 1024) + b"\n%%EOF\n"
    for r, d in s9.items():
        put("observer", r, d)
    sync("observer")
    proc = subprocess.Popen([OB, "sync", "--path", str(MIRROR), "--continuous"], env=env_for("mirror"),
                            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    peak, children, done = 0, set(), False
    t0 = time.monotonic()
    while time.monotonic() - t0 < 120:
        try:
            status = pathlib.Path(f"/proc/{proc.pid}/status").read_text()
            peak = max(peak, int(next(l.split()[1] for l in status.splitlines() if l.startswith("VmHWM:"))))
            kids = subprocess.run(["pgrep", "-P", str(proc.pid)], capture_output=True, text=True).stdout.split()
            children.update(kids)
        except (FileNotFoundError, StopIteration):
            break
        if all(digest("mirror", r) == hashlib.sha256(d).hexdigest() for r, d in s9.items()):
            done = True
            break
        time.sleep(0.5)
    took = time.monotonic() - t0
    proc.send_signal(signal.SIGTERM)
    try:
        rc9 = proc.wait(timeout=20)
        exited = True
    except subprocess.TimeoutExpired:
        proc.kill()
        rc9 = proc.wait()
        exited = False
    left = [c for c in children if pathlib.Path(f"/proc/{c}").exists()]
    result("S9a", "持续模式收齐 21 个文件（含 20 MiB）", "PASS" if done else "FAIL", f"用时 {took:.1f}s")
    result("S9b", "常驻内存峰值与进程树", "OBSERVED",
           f"VmHWM 峰值 {peak / 1024:.1f} MiB；子进程={sorted(children) or '无'}；SIGTERM 后 20s 内退出={exited} rc={rc9}；残留子进程={left or '无'}")

    r9 = "S9/中途终止.pdf"
    d9 = b"%PDF-1.4\n" + os.urandom(50 * 1024 * 1024) + b"\n%%EOF\n"
    put("observer", r9, d9)
    sync("observer")
    proc = subprocess.Popen([OB, "sync", "--path", str(MIRROR), "--continuous"], env=env_for("mirror"),
                            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    time.sleep(2.0)
    proc.kill()
    proc.wait()
    partial = rel("mirror", r9).stat().st_size if rel("mirror", r9).is_file() else None
    converge("mirror", lambda: digest("mirror", r9) == hashlib.sha256(d9).hexdigest())
    intact = digest("mirror", r9) == hashlib.sha256(d9).hexdigest()
    strays = [f for f in listing("mirror", "S9") if f not in s9 and f != r9]
    result("S9c", "下载中途 SIGKILL 后重开能补齐且不留半截文件", "PASS" if (intact and not strays) else "FAIL",
           f"杀掉时本地大小={partial}；重开后完整={intact}；多余文件={strays or '无'}")

    log("## 未做项")
    log("   断网（需 root 网络命名空间）、大附件阈值 200000000/208666624/209715200 字节（耗配额，另跑）")
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
