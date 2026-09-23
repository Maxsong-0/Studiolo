"""私有副本存储：完整验证、互斥发布、原子回执及两代保留。"""
import fcntl
import os
import re
import time
import uuid

from replica_cleanup import names_bounded, remove_tree
from replica_common import (MAX_ENTRIES, MAX_FILE, MAX_MANIFEST, Invalid, check_node,
                            descend, digest, encode, integer, load_json, manifest,
                            open_dir, open_root, read_file, require, write_file)


def now_ms():
    return time.time_ns() // 1_000_000


class BaseMismatch(Exception):
    """增量／确认请求所依据的基线不是本端现代次，或现代次已损坏：不发布，请源端改发整份。

    corrupt=True 表示现代次内容与其清单不符，须如实记为 validation_failed。
    """

    def __init__(self, corrupt):
        super().__init__("base_mismatch")
        self.corrupt = corrupt


def receipt_valid(receipt, attempt):
    require(isinstance(receipt, dict) and set(receipt) == {
        "generation", "source_digest", "source_observed_at_ms", "received_at_ms"})
    require(isinstance(receipt["generation"], str) and re.fullmatch("[a-f0-9]{32}", receipt["generation"]))
    require(isinstance(receipt["source_digest"], str) and re.fullmatch("[a-f0-9]{64}", receipt["source_digest"]))
    source, received = receipt["source_observed_at_ms"], receipt["received_at_ms"]
    require(integer(source) and integer(received) and 0 < source <= received <= attempt)


def state_valid(value):
    require(isinstance(value, dict) and set(value) == {
        "schema_version", "status", "last_attempt_at_ms", "last_error", "receipt"})
    require(type(value["schema_version"]) is int and value["schema_version"] == 1)
    require(integer(value["last_attempt_at_ms"]) and 0 < value["last_attempt_at_ms"] <= now_ms() + 30_000)
    if value["status"] == "ready":
        require(value["last_error"] is None and value["receipt"] is not None)
    else:
        require(value["status"] == "failed" and value["last_error"] in ("transfer_failed", "validation_failed"))
    if value["receipt"] is not None:
        receipt_valid(value["receipt"], value["last_attempt_at_ms"])
    return value


def verify_tree(root, expected):
    require(set(names_bounded(root, 3)) == {"manifest.json", "vault"})
    raw, _ = read_file(root, "manifest.json", MAX_MANIFEST)
    _, recorded = manifest(raw)
    require(recorded == expected)
    visited = set()

    def walk(fd, prefix):
        names = names_bounded(fd)
        require(len(names) <= MAX_ENTRIES + 16)
        for name in names:
            path = prefix + "/" + name if prefix else name
            require(path in expected and path not in visited)
            visited.add(path)
            kind, size, sha = expected[path]
            if kind == "dir":
                child = open_dir(fd, name)
                try:
                    walk(child, path)
                finally:
                    os.close(child)
            else:
                raw_file, _ = read_file(fd, name, MAX_FILE)
                require(len(raw_file) == size and digest(raw_file) == sha)
    vault = open_dir(root, "vault")
    try:
        visited.add("vault")
        walk(vault, "vault")
    finally:
        os.close(vault)
    require(visited == set(expected))
    return digest(raw)



class Store:
    def __init__(self, path):
        self.path, self.fds = path, []
        self.previous = None
        try:
            self.root = self.keep(open_root(path, create=True, private=True))
            self.lock = self.keep(os.open("sync.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW,
                                          0o600, dir_fd=self.root))
            check_node(os.fstat(self.lock), False, private=True)
            require(os.fstat(self.lock).st_dev == os.fstat(self.root).st_dev)
            fcntl.flock(self.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
            self.control = self.keep(open_dir(self.root, "control", create=True))
            self.releases = self.keep(open_dir(self.root, "releases", create=True))
            try:
                raw, _ = read_file(self.control, "state.json", 8192)
                self.previous = state_valid(load_json(raw))
            except FileNotFoundError:
                pass
            for name in names_bounded(self.control, 32):
                if name == "state.json":
                    continue
                require(re.fullmatch(r"\.state-[a-f0-9]{32}", name))
                read_file(self.control, name, 8192)
                os.unlink(name, dir_fd=self.control)
            for name in names_bounded(self.releases, 64):
                require(re.fullmatch("[a-f0-9]{32}", name))
            for name in names_bounded(self.root, 32):
                if name in {"sync.lock", "control", "releases"}:
                    continue
                require(re.fullmatch(r"\.stage-[a-f0-9]{32}", name))
                remove_tree(self.root, name)
        except BaseException:
            self.close()
            raise

    def keep(self, fd):
        self.fds.append(fd)
        return fd

    def close(self):
        for fd in reversed(self.fds):
            os.close(fd)
        self.fds.clear()

    def check_binding(self):
        lock = os.stat("sync.lock", dir_fd=self.root, follow_symlinks=False)
        check_node(lock, False, private=True)
        require((lock.st_dev, lock.st_ino) == (os.fstat(self.lock).st_dev, os.fstat(self.lock).st_ino))
        fd = open_root(self.path, private=True)
        try:
            require((os.fstat(fd).st_dev, os.fstat(fd).st_ino) ==
                    (os.fstat(self.root).st_dev, os.fstat(self.root).st_ino))
        finally:
            os.close(fd)
        for name, original in (("control", self.control), ("releases", self.releases)):
            fd = open_dir(self.root, name)
            try:
                require(os.fstat(fd).st_ino == os.fstat(original).st_ino)
            finally:
                os.close(fd)

    def state(self, status, receipt, error=None):
        self.check_binding()
        attempt = now_ms()
        if self.previous:
            require(attempt >= self.previous["last_attempt_at_ms"])
        if receipt:
            require(attempt >= receipt["received_at_ms"])
        value = {"schema_version": 1, "status": status, "last_attempt_at_ms": attempt,
                 "last_error": error, "receipt": receipt}
        state_valid(value)
        name = ".state-" + uuid.uuid4().hex
        write_file(self.control, name, encode(value))
        try:
            os.replace(name, "state.json", src_dir_fd=self.control, dst_dir_fd=self.control)
            os.fsync(self.control)
            self.previous = value
        finally:
            try:
                os.unlink(name, dir_fd=self.control)
            except FileNotFoundError:
                pass

    def failed(self, reason):
        self.state("failed", self.previous["receipt"] if self.previous else None, reason)

    def base(self, base_digest):
        """打开请求所依据的现代次，返回 (目录 fd, 基线 expected)；此处只核清单，正文由调用方按需核对。"""
        old = self.previous["receipt"] if self.previous else None
        if old is None or old["source_digest"] != base_digest:
            raise BaseMismatch(corrupt=False)
        try:
            prior = open_dir(self.releases, old["generation"])
        except (FileNotFoundError, Invalid):
            raise BaseMismatch(corrupt=True)
        try:
            raw, _ = read_file(prior, "manifest.json", MAX_MANIFEST)
            if digest(raw) != base_digest:
                raise BaseMismatch(corrupt=True)
            _, expected = manifest(raw)
            return prior, expected
        except (FileNotFoundError, Invalid):
            os.close(prior)
            raise BaseMismatch(corrupt=True)
        except BaseException:
            os.close(prior)
            raise

    def confirm(self, prior, sha, observed, expected):
        """内容未变：核对现代次完好后只刷新回执时间，不建新代次、不写正文。"""
        received = now_ms()
        require(0 < observed <= received)
        old = self.previous["receipt"]
        require(old["source_digest"] == sha)
        require(observed >= old["source_observed_at_ms"] and received >= old["received_at_ms"])
        try:
            intact = verify_tree(prior, expected) == sha
        except (FileNotFoundError, Invalid):
            intact = False
        if not intact:
            raise BaseMismatch(corrupt=True)
        receipt = {"generation": old["generation"], "source_digest": sha,
                   "source_observed_at_ms": observed, "received_at_ms": received}
        self.state("ready", receipt)
        return receipt

    def copy(self, prior, stage, path, entry):
        """把与基线相同的文件从现代次复制进 stage；读到的与清单不符即视为现代次损坏。"""
        kind, size, sha = entry
        require(kind == "file")
        parent, name = path.rsplit("/", 1)
        try:
            source = descend(prior, parent)
            try:
                raw, _ = read_file(source, name, MAX_FILE)
            finally:
                os.close(source)
        except (FileNotFoundError, Invalid):
            raise BaseMismatch(corrupt=True)
        if len(raw) != size or digest(raw) != sha:
            raise BaseMismatch(corrupt=True)
        target = descend(stage, parent)
        try:
            write_file(target, name, raw)
            os.fsync(target)
        finally:
            os.close(target)

    def publish(self, stage_name, stage_fd, sha, observed, expected):
        received = now_ms()
        require(0 < observed <= received)
        require(verify_tree(stage_fd, expected) == sha)
        old = self.previous["receipt"] if self.previous else None
        if old:
            require(observed >= old["source_observed_at_ms"] and received >= old["received_at_ms"])
        generation = uuid.uuid4().hex
        reused = False
        if old and old["source_digest"] == sha:
            prior = open_dir(self.releases, old["generation"])
            try:
                try:
                    intact = verify_tree(prior, expected) == sha
                except Invalid:
                    intact = False
                if intact:
                    generation, reused = old["generation"], True
            finally:
                os.close(prior)
        self.check_binding()
        if not reused:
            st = os.stat(stage_name, dir_fd=self.root, follow_symlinks=False)
            require((st.st_dev, st.st_ino) == (os.fstat(stage_fd).st_dev, os.fstat(stage_fd).st_ino))
            os.rename(stage_name, generation, src_dir_fd=self.root, dst_dir_fd=self.releases)
            os.fsync(self.releases)
            os.fsync(self.root)
        receipt = {"generation": generation, "source_digest": sha,
                   "source_observed_at_ms": observed, "received_at_ms": received}
        self.state("ready", receipt)
        keep = {generation}
        if old:
            keep.add(old["generation"])
        # 心跳不淘汰上一代；真正发布新代次才移除更旧版本。
        if not reused:
            for name in names_bounded(self.releases, 64):
                if name not in keep:
                    require(re.fullmatch("[a-f0-9]{32}", name))
                    remove_tree(self.releases, name)
            os.fsync(self.releases)
        return receipt, reused
