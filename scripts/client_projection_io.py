"""私有投影目录的有界、逐分量不跟随链接读写。"""
import os

from client_projection_schema import MAX_MANIFEST, validate_bundle, validate_manifest
from replica_common import (DIR_FLAGS, check_node, digest, encode, fingerprint, load_json, open_root,
                            read_file, require, write_file)


def read_bundle(path):
    root = open_root(os.fspath(path), private=True)
    try:
        initial = fingerprint(os.fstat(root))
        raw, manifest_stat = read_file(root, "manifest.json", MAX_MANIFEST)
        manifest = validate_manifest(load_json(raw))
        declared = {f["path"]: f for f in manifest["files"]}
        expected = {f["path"] for f in manifest["files"]} | {"manifest.json"}
        dirs = {"/".join(p.split("/")[:n]) for p in expected
                for n in range(1, len(p.split("/")))}
        output = {}

        def walk(fd, prefix=""):
            before = fingerprint(os.fstat(fd))
            with os.scandir(fd) as entries:
                names = []
                for entry in entries:
                    names.append(entry.name)
                    require(len(names) <= len(expected) + len(dirs))
            for name in names:
                relative = prefix + name
                require(relative in expected or relative in dirs)
                if relative in dirs:
                    child = os.open(name, DIR_FLAGS, dir_fd=fd)
                    try:
                        check_node(os.fstat(child), True, private=True)
                        require(os.fstat(child).st_dev == os.fstat(root).st_dev)
                        child_before = fingerprint(os.fstat(child))
                        walk(child, relative + "/")
                        require(child_before == fingerprint(os.stat(name, dir_fd=fd, follow_symlinks=False)))
                    finally:
                        os.close(child)
                elif relative != "manifest.json":
                    item = declared[relative]
                    body = read_file(fd, name, item["bytes"])[0]
                    require(len(body) == item["bytes"] and digest(body) == item["sha256"])
                    output[relative] = body
            require(before == fingerprint(os.fstat(fd)))

        walk(root)
        require(manifest_stat == fingerprint(os.stat("manifest.json", dir_fd=root, follow_symlinks=False)))
        require(initial == fingerprint(os.fstat(root)))
        reopened = open_root(os.fspath(path), private=True)
        try:
            require(initial == fingerprint(os.fstat(reopened)))
        finally:
            os.close(reopened)
        return validate_bundle({"manifest": manifest, "files": output})
    finally:
        os.close(root)


def write_contents(root, bundle):
    """调用方持有一个新建且尚未公开的700目录；失败不删除残留证据。"""
    validate_bundle(bundle)
    for path, raw in sorted(bundle["files"].items()):
        parts = path.split("/")
        fd = os.dup(root)
        try:
            for part in parts[:-1]:
                try:
                    os.mkdir(part, 0o700, dir_fd=fd)
                    os.fsync(fd)
                except FileExistsError:
                    pass
                child = os.open(part, DIR_FLAGS, dir_fd=fd)
                check_node(os.fstat(child), True, private=True)
                require(os.fstat(child).st_dev == os.fstat(root).st_dev)
                os.close(fd)
                fd = child
            write_file(fd, parts[-1], raw)
            os.fsync(fd)
        finally:
            os.close(fd)
    write_file(root, "manifest.json", encode(bundle["manifest"]) + b"\n")
    os.fsync(root)


def write_bundle(path, bundle):
    validate_bundle(bundle)
    path = os.fspath(path)
    require(os.path.isabs(path) and all(p not in ("", ".", "..") for p in path.split("/")[1:]))
    parent_path, name = os.path.split(path)
    parent = open_root(parent_path)
    try:
        os.mkdir(name, 0o700, dir_fd=parent)
        root = os.open(name, DIR_FLAGS, dir_fd=parent)
        try:
            write_contents(root, bundle)
        finally:
            os.close(root)
        os.fsync(parent)
        result = read_bundle(path)
        require(result == bundle)
        return result
    finally:
        os.close(parent)
