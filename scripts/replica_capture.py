"""稳定地双读白名单；源头变化或危险节点使整批失败。"""
import io
import os
import stat
import tarfile
import tempfile
import time

from replica_common import (ALLOW, MAX_ENTRIES, MAX_FILE, MAX_MANIFEST, MAX_NOTES_TOTAL, MAX_TOTAL,
                            check_node, descend, digest, encode, fingerprint,
                            logical_path_ok, manifest, open_root, read_file, require)


def scan(root_fd, keep=True):
    entries, bodies, snapshots, folded = [], {}, {}, set()
    total, inspected, notes_total = 0, 0, 0

    def walk(fd, path):
        nonlocal total, inspected, notes_total
        before = fingerprint(os.fstat(fd))
        snapshots[path] = before
        names = []
        with os.scandir(fd) as iterator:
            for node in iterator:
                inspected += 1
                require(inspected <= MAX_ENTRIES)
                if not node.name.startswith("."):
                    names.append(node.name)
        names.sort()
        for name in names:
            p = path + "/" + name
            logical_path_ok(p)
            require(p.lower() not in folded and len(entries) < MAX_ENTRIES)
            folded.add(p.lower())
            st = os.stat(name, dir_fd=fd, follow_symlinks=False)
            if stat.S_ISDIR(st.st_mode):
                child = descend(fd, name, private=False)
                try:
                    require(fingerprint(st) == fingerprint(os.fstat(child)))
                    entries.append({"path": p, "kind": "dir", "bytes": 0, "sha256": None})
                    walk(child, p)
                    require(fingerprint(st) == fingerprint(os.stat(name, dir_fd=fd, follow_symlinks=False)))
                finally:
                    os.close(child)
            else:
                check_node(st, False)
                raw, fp = read_file(fd, name, MAX_MANIFEST if p.lower().endswith(".md") else MAX_FILE, False)
                require(fp == fingerprint(st))
                total += len(raw)
                require(total <= MAX_TOTAL)
                if p.lower().endswith(".md"):
                    raw.decode("utf-8")
                    notes_total += len(raw)
                    require(notes_total <= MAX_NOTES_TOTAL)
                entries.append({"path": p, "kind": "file", "bytes": len(raw), "sha256": digest(raw)})
                snapshots[p] = fp
                if keep:
                    bodies[p] = raw
        require(before == fingerprint(os.fstat(fd)))

    for allow in ALLOW:
        fd = descend(root_fd, allow, private=False)
        try:
            walk(fd, allow)
        finally:
            os.close(fd)
    entries.sort(key=lambda item: item["path"])
    return entries, bodies, snapshots


def capture(path, between_scans=None):
    root = open_root(path)
    try:
        before = fingerprint(os.fstat(root))
        entries, bodies, snapshots = scan(root)
        if between_scans is not None:
            between_scans()
        second, _, latest = scan(root, keep=False)
        require(entries == second and snapshots == latest)
        require(before == fingerprint(os.fstat(root)))
        # 重新解析绝对路径，捕获祖先/根在扫描期间被替换的情况。
        check = open_root(path)
        try:
            require(before == fingerprint(os.fstat(check)))
        finally:
            os.close(check)
        raw = encode({"schema_version": 1, "allow": ALLOW, "entries": entries})
        _, expected = manifest(raw)
        observed = time.time_ns() // 1_000_000
        return raw, bodies, expected, observed
    finally:
        os.close(root)


def transfer_mode(captured, base):
    """base 为 (基线清单摘要, 基线 expected) 或 None。"""
    if base is None:
        return "full"
    return "confirm" if base[0] == digest(captured[0]) else "delta"


def archive(captured, base=None):
    """整份：全部条目；确认：只有清单与请求；增量：全部目录＋与基线不同的文件（ADR-024）。"""
    raw, bodies, expected, observed = captured
    mode = transfer_mode(captured, base)
    base_digest, base_expected = base if base is not None else (None, None)
    output = tempfile.TemporaryFile()
    try:
        with tarfile.open(fileobj=output, mode="w", format=tarfile.PAX_FORMAT) as tar:
            def put(path, body=None):
                info = tarfile.TarInfo(path)
                info.mode = 0o700 if body is None else 0o600
                info.type = tarfile.DIRTYPE if body is None else tarfile.REGTYPE
                info.size = 0 if body is None else len(body)
                tar.addfile(info, None if body is None else io.BytesIO(body))
            put("manifest.json", raw)
            put("request.json", encode({"schema_version": 2, "source_observed_at_ms": observed,
                                        "base_digest": base_digest}))
            if mode != "confirm":
                for path, entry in sorted(expected.items()):
                    if entry[0] == "dir":
                        put(path)
                    elif mode == "full" or base_expected.get(path) != entry:
                        put(path, bodies[path[len("vault/"):]])
        output.seek(0)
        return output
    except BaseException:
        output.close()
        raise
