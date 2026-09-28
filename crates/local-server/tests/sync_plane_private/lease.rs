//! `plane.lock` 的释放语义：确定性复制同一个持锁 FD，模拟并发 fork 在 exec 之前继承的描述符；
//! 不声称复现特定的 fork 调度（真实竞态见 2026-09-28 main CI 的偶发失败）。

use super::PlaneStateStore;
use crate::sync_plane::PlaneError;
use std::fs;
use std::os::unix::fs::PermissionsExt;

fn plane_root() -> tempfile::TempDir {
    let dir = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    dir
}

#[test]
fn drop_releases_the_lease_even_while_an_inherited_copy_is_alive() {
    let root = plane_root();
    let store = PlaneStateStore::open(root.path()).unwrap();
    // 同一打开文件描述的副本，等同于并发 fork 的子进程在 exec 之前继承到的那一份。
    let inherited = store.lease.try_clone().unwrap();
    assert!(matches!(
        PlaneStateStore::open(root.path()),
        Err(PlaneError::Storage)
    ));
    drop(store);
    // 不重试、不睡眠：副本还在时下一个持有者也必须立刻拿得到。
    let next = PlaneStateStore::open(root.path()).unwrap();
    assert!(matches!(
        PlaneStateStore::open(root.path()),
        Err(PlaneError::Storage)
    ));
    // 旧副本关闭不能解除新持有者在另一个打开文件描述上的锁。
    drop(inherited);
    assert!(matches!(
        PlaneStateStore::open(root.path()),
        Err(PlaneError::Storage)
    ));
    drop(next);
    assert!(PlaneStateStore::open(root.path()).is_ok());
}
