"""协议专属私有树的有界崩溃恢复；先完整检查，再按同一节点清理。"""
import os
import stat

from replica_common import (DIR_FLAGS, MAX_ENTRIES, MAX_FILE, MAX_MANIFEST,
                            MAX_TOTAL, check_node, fingerprint, path_ok, require)


def names_bounded(fd, limit=MAX_ENTRIES + 16):
    names = []
    with os.scandir(fd) as iterator:
        for node in iterator:
            require(len(names) < limit)
            names.append(node.name)
    return names


def remove_tree(parent, name):
    """名字由 Store 确认专属；不追踪链接，不跨设备，不触碰未知目录。"""
    inventory = {}
    total = 0

    def collect(base, child, prefix):
        nonlocal total
        require(len(prefix.split("/")) <= 66 and len(inventory) < MAX_ENTRIES + 32)
        st = os.stat(child, dir_fd=base, follow_symlinks=False)
        directory = stat.S_ISDIR(st.st_mode)
        check_node(st, directory, private=True)
        require(st.st_dev == os.fstat(base).st_dev)
        inventory[prefix] = fingerprint(st)
        if directory:
            fd = os.open(child, DIR_FLAGS, dir_fd=base)
            try:
                require(fingerprint(os.fstat(fd)) == fingerprint(st))
                for entry in names_bounded(fd):
                    path_ok(entry)
                    collect(fd, entry, prefix + "/" + entry)
                require(fingerprint(os.fstat(fd)) == fingerprint(st))
            finally:
                os.close(fd)
        else:
            require(st.st_size <= MAX_FILE)
            total += st.st_size
            require(total <= MAX_TOTAL + MAX_MANIFEST)

    def erase(base, child, prefix):
        st = os.stat(child, dir_fd=base, follow_symlinks=False)
        require(fingerprint(st) == inventory[prefix])
        if stat.S_ISDIR(st.st_mode):
            fd = os.open(child, DIR_FLAGS, dir_fd=base)
            try:
                require(fingerprint(os.fstat(fd)) == inventory[prefix])
                for entry in names_bounded(fd):
                    path = prefix + "/" + entry
                    require(path in inventory)
                    erase(fd, entry, path)
                bound = os.stat(child, dir_fd=base, follow_symlinks=False)
                require((bound.st_dev, bound.st_ino) == (st.st_dev, st.st_ino))
            finally:
                os.close(fd)
            os.rmdir(child, dir_fd=base)
        else:
            os.unlink(child, dir_fd=base)
    collect(parent, name, name)
    erase(parent, name, name)
    os.fsync(parent)
