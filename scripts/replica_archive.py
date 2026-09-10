"""有界 tar 流读取；显式处理小型 PAX path，不执行通用解包。"""
import codecs
import hashlib
import os
import tarfile

from replica_common import (MAX_ENTRIES, MAX_MANIFEST, MAX_TOTAL, descend, digest,
                            integer, load_json, manifest, path_ok, require, write_file)

MAX_WIRE = 256 * 1024 * 1024
MAX_HEADERS = 2 * (MAX_ENTRIES + 16)


class Reader:
    def __init__(self, stream):
        self.stream, self.count = stream, 0

    def read(self, size):
        require(0 <= size <= MAX_MANIFEST)
        data = bytearray()
        while len(data) < size:
            chunk = self.stream.read(size - len(data))
            if not chunk:
                raise EOFError("transfer_failed")
            self.count += len(chunk)
            require(self.count <= MAX_WIRE)
            data.extend(chunk)
        return bytes(data)

    def padding(self, size):
        amount = (-size) % 512
        if amount:
            require(not any(self.read(amount)))

    def finish(self):
        require(self.read(512) == bytes(512))
        while True:
            tail = self.stream.read(65536)
            if not tail:
                break
            self.count += len(tail)
            require(self.count <= MAX_WIRE and not any(tail))


def pax_path(raw):
    require(0 < len(raw) <= 8192)
    space = raw.find(b" ")
    require(0 < space <= 5 and raw[:space].isdigit())
    require(int(raw[:space]) == len(raw) and raw.endswith(b"\n"))
    require(raw[space + 1:].startswith(b"path="))
    path = raw[space + 6:-1].decode("utf-8")
    if path.endswith("/"):
        path = path[:-1]
    path_ok(path)
    return path


def receive(stream, stage):
    reader, seen = Reader(stream), set()
    pending = None
    raw_manifest = request = expected = None
    total = 0
    for _ in range(MAX_HEADERS):
        header = reader.read(512)
        if header == bytes(512):
            require(pending is None)
            reader.finish()
            break
        info = tarfile.TarInfo.frombuf(header, "utf-8", "strict")
        require(integer(info.size) and info.size >= 0)
        if info.type == tarfile.XHDTYPE:
            require(pending is None and 0 < info.size <= 8192)
            pending = pax_path(reader.read(info.size))
            reader.padding(info.size)
            continue
        require(info.type in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE))
        path = pending if pending is not None else info.name
        pending = None
        if info.isdir() and path.endswith("/"):
            path = path[:-1]
        path_ok(path)
        require(path not in seen and len(seen) <= MAX_ENTRIES + 16)
        seen.add(path)
        if path == "manifest.json":
            require(raw_manifest is None and len(seen) == 1 and info.isfile() and info.size <= MAX_MANIFEST)
            raw_manifest = reader.read(info.size)
            _, expected = manifest(raw_manifest)
            write_file(stage, path, raw_manifest)
        elif path == "request.json":
            require(request is None and len(seen) == 2 and info.isfile() and info.size <= 4096)
            request = load_json(reader.read(info.size))
            require(isinstance(request, dict) and set(request) == {"schema_version", "source_observed_at_ms"})
            require(type(request["schema_version"]) is int and request["schema_version"] == 1)
            require(integer(request["source_observed_at_ms"]) and request["source_observed_at_ms"] > 0)
        else:
            require(expected is not None and request is not None and path in expected)
            kind, size, sha = expected[path]
            require(info.size == size and (info.isdir() if kind == "dir" else info.isfile()))
            if kind == "dir":
                fd = descend(stage, path, create=True)
                os.fsync(fd)
                os.close(fd)
            else:
                parent, name = path.rsplit("/", 1)
                fd = descend(stage, parent)
                try:
                    out = os.open(name, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW,
                                  0o600, dir_fd=fd)
                    try:
                        remaining, actual = size, hashlib.sha256()
                        decoder = codecs.getincrementaldecoder("utf-8")() if path.lower().endswith(".md") else None
                        with os.fdopen(out, "wb", closefd=False) as target:
                            while remaining:
                                chunk = reader.read(min(65536, remaining))
                                target.write(chunk)
                                actual.update(chunk)
                                if decoder is not None:
                                    decoder.decode(chunk)
                                remaining -= len(chunk)
                            target.flush()
                            os.fsync(out)
                        require(actual.hexdigest() == sha)
                        if decoder is not None:
                            decoder.decode(b"", final=True)
                    finally:
                        os.close(out)
                    os.fsync(fd)
                finally:
                    os.close(fd)
                total += size
                require(total <= MAX_TOTAL)
        reader.padding(info.size)
    else:
        require(False)
    require(expected is not None and request is not None)
    require(seen == set(expected) | {"manifest.json", "request.json"})
    os.fsync(stage)
    return digest(raw_manifest), request["source_observed_at_ms"], expected
