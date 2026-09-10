"""只读计划、私有客户端上下文发布及离线完整性检查。"""
import fcntl
import os
import uuid

from client_projection_fs import View, bound_directory, directory, identity, pointer, write_path
from client_projection_schema import read_bundle, validate_bundle
from replica_cleanup import names_bounded
from replica_common import check_node, encode, fingerprint, require, write_file


def _read(view, bundle_id):
    view.binding()
    fd = directory(view.releases, bundle_id)
    try:
        result = read_bundle(os.path.join(view.path, "releases", bundle_id))
        require(result["manifest"]["bundle_id"] == bundle_id)
        bound_directory(view.releases, bundle_id, fd)
        view.binding()
        return result
    finally:
        os.close(fd)


def _current(view):
    if view.empty:
        return None, (None, None)
    before = pointer(view.root)
    if before[0] is None:
        # 有批次却缺失指针时无法区分用户删除与首轮中断，保守保留并拒绝继续发布。
        require(not names_bounded(view.releases, 1))
    result = _read(view, before[0]) if before[0] else None
    require(pointer(view.root) == before)
    return result, before


def _diff(old, new):
    previous = {item["path"]: item["sha256"] for item in old["manifest"]["files"]} if old else {}
    following = {item["path"]: item["sha256"] for item in new["manifest"]["files"]}
    return [{"path": path, "old_sha256": previous.get(path), "new_sha256": following.get(path)}
            for path in sorted(previous.keys() | following.keys())]


def _plan(old, bundle):
    same = old and old["manifest"]["projection_key"] == bundle["manifest"]["projection_key"]
    return {"status": "unchanged" if same else "changed" if old else "create",
            "current_bundle_id": old["manifest"]["bundle_id"] if old else None,
            "bundle_id": bundle["manifest"]["bundle_id"], "files": _diff(old, bundle)}


def check(target):
    """只检本地完整性；ok 不代表来源仍新鲜，不发起网络请求。"""
    known_id = None
    try:
        with View(target) as view:
            known_id = pointer(view.root)[0] if not view.empty else None
            old, _ = _current(view)
            view.binding()
            return {"status": "ok" if old else "empty",
                    "bundle_id": old["manifest"]["bundle_id"] if old else None,
                    "manifest": old["manifest"] if old else None}
    except FileNotFoundError:
        # 只有目标本身不存在才算 empty；缺失已管理文件属于漂移。
        if not os.path.lexists(target):
            return {"status": "empty", "bundle_id": None, "manifest": None}
    except (OSError, ValueError, KeyError, TypeError):
        pass
    return {"status": "local_modified", "bundle_id": known_id, "manifest": None}


def plan(target, bundle):
    """纯读取，不创建目标、目录或锁；只回传路径与摘要。"""
    validate_bundle(bundle)
    try:
        with View(target) as view:
            old, _ = _current(view)
            view.binding()
            return _plan(old, bundle)
    except FileNotFoundError:
        if not os.path.lexists(target):
            return _plan(None, bundle)
    except (OSError, ValueError, KeyError, TypeError):
        pass
    return {"status": "conflict", "current_bundle_id": None,
            "bundle_id": bundle["manifest"]["bundle_id"], "files": _diff(None, bundle)}


def _lock(view):
    # 从空目录初始化时只增加专属锁；未知非空目录在 View 中已拒绝。
    view.lock = os.open("projection.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW,
                        0o600, dir_fd=view.root)
    check_node(os.fstat(view.lock), False, private=True)
    require(os.fstat(view.lock).st_size == 0 and identity(view.lock)[0] == identity(view.root)[0])
    fcntl.flock(view.lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
    view.binding()
    view.initialize()
    view.binding()


def _prepare(view, bundle):
    bundle_id = bundle["manifest"]["bundle_id"]
    try:
        # 新代次本身是未激活的暂存目录；mkdir 不覆盖任何已有文件或目录。
        os.mkdir(bundle_id, 0o700, dir_fd=view.releases)
        os.fsync(view.releases)
    except FileExistsError:
        require(_read(view, bundle_id) == bundle)
        return
    stage = directory(view.releases, bundle_id)
    try:
        for path, raw in sorted(bundle["files"].items()):
            write_path(stage, path, raw)
        write_file(stage, "manifest.json", encode(bundle["manifest"]))
        os.fsync(stage)
        bound_directory(view.releases, bundle_id, stage)
        require(_read(view, bundle_id) == bundle)
    finally:
        # 失败保留暂存代次，绝不递归删除可能已被用户修改的文件。
        os.close(stage)


def _activate(view, bundle, before, old):
    view.binding()
    require(pointer(view.root) == before)
    if old:
        require(_read(view, before[0]) == old)
    require(_read(view, bundle["manifest"]["bundle_id"]) == bundle)
    view.binding()
    require(pointer(view.root) == before)
    name = ".current-" + uuid.uuid4().hex
    os.symlink("releases/" + bundle["manifest"]["bundle_id"], name, dir_fd=view.root)
    created = fingerprint(os.stat(name, dir_fd=view.root, follow_symlinks=False))
    try:
        view.binding(temporary=name)
        require(pointer(view.root) == before)
        require(fingerprint(os.stat(name, dir_fd=view.root, follow_symlinks=False)) == created)
        # 最后一步只切换当前批次指针，读者不会看到逐文件替换的中间态。
        os.replace(name, "current", src_dir_fd=view.root, dst_dir_fd=view.root)
        try:
            os.fsync(view.root)
        except OSError:
            # replace 已提交；不能将持久化不确定冒称为仍在旧版，也不反向覆盖。
            return False
        return True
    finally:
        # 原子替换失败时仅清理由本调用创建、仍指向预期代次的临时链接。
        try:
            if fingerprint(os.stat(name, dir_fd=view.root, follow_symlinks=False)) == created:
                os.unlink(name, dir_fd=view.root)
        except FileNotFoundError:
            pass


def apply(target, bundle):
    """持排他锁重新规划；本地漂移、未知文件或根重绑定都拒绝，不提供 force。"""
    validate_bundle(bundle)
    with View(target, create=True) as view:
        _lock(view)
        old, before = _current(view)
        result = _plan(old, bundle)
        if result["status"] == "unchanged":
            view.binding()
            require(pointer(view.root) == before)
            require(_read(view, before[0]) == old)
            result["bundle_id"] = old["manifest"]["bundle_id"]
            return result
        _prepare(view, bundle)
        durable = _activate(view, bundle, before, old)
        result["current_bundle_id"] = bundle["manifest"]["bundle_id"]
        if not durable:
            result["status"] = "activated_durability_unknown"
        return result
