//! 发布残留的收口契约：强杀或断电留下的临时文件，权威记录核过之后才丢；核不过的一个字节不动。

use super::launcher_fake::FakeLauncher;
use super::rebind_store::Store;
use crate::linux::verify_backend_at;
use crate::owned_run::*;
use crate::store::{FilePublisher, paths};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
};

fn open(store: &Store) -> Result<SyncTransportOwner<'_>> {
    let launcher = FakeLauncher::start();
    let backend =
        verify_backend_at(&store.approval, launcher.socket()).expect("假 launcher 探测通过");
    SyncTransportOwner::open_existing(
        &store.approval,
        PinnedRuntime::from_approval(&store.approval).expect("固定运行时可取"),
        backend,
    )
}

fn root(store: &Store) -> PathBuf {
    store.approval.roots().private_root
}

fn residue(store: &Store) -> PathBuf {
    root(store).join(paths::STATE_PUBLISH_TEMP)
}

fn leave(store: &Store, bytes: &[u8]) {
    write_private(&residue(store), bytes);
}

fn write_private(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes).expect("可以摆残留");
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).expect("可以设权限");
}

/// 下一代的完整记录：断电后改名没落盘、临时文件却写全了，就是这个样子。
fn next_record(store: &Store) -> Vec<u8> {
    let mut state = state::decode(&store.bytes()).expect("权威记录可解析");
    state.generation += 1;
    state::encode(&state).expect("可以编码")
}

#[test]
fn opening_the_owner_discards_a_residue_once_the_authoritative_record_is_verified() {
    for shape in ["empty", "half", "complete"] {
        let store = Store::new();
        let old = store.bytes();
        let next = next_record(&store);
        leave(
            &store,
            match shape {
                "empty" => &[],
                "half" => &next[..next.len() / 2],
                _ => &next,
            },
        );
        let owner = open(&store).expect("形态合规的残留不挡启动");
        assert!(owner.discarded_residue(), "{shape}");
        assert!(!residue(&store).exists(), "{shape}");
        assert_eq!(store.bytes(), old, "{shape}：权威记录一个字节不变");
        // 丢掉的是没提交的那一份：状态仍是旧的一代。
        assert_eq!(owner.inspect().expect("可以读状态").generation, 1);
        drop(owner);
        let again = open(&store).expect("再开没有残留");
        assert!(!again.discarded_residue());
    }
}

#[test]
fn an_unexpected_residue_shape_is_refused_and_nothing_changes() {
    for shape in ["symlink", "directory", "hardlink", "mode", "oversize"] {
        let store = Store::new();
        let old = store.bytes();
        let path = residue(&store);
        let elsewhere = root(&store).with_file_name("elsewhere");
        match shape {
            "symlink" => {
                write_private(&elsewhere, b"target");
                symlink(&elsewhere, &path).expect("可以摆链接");
            }
            "directory" => fs::create_dir(&path).expect("可以摆目录"),
            "hardlink" => {
                leave(&store, b"x");
                fs::hard_link(&path, &elsewhere).expect("可以摆硬链接");
            }
            "mode" => {
                leave(&store, b"x");
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).expect("可以改权限");
            }
            _ => leave(&store, &vec![b'x'; STATE_MAX_BYTES + 1]),
        }
        assert_eq!(
            open(&store).err(),
            Some(TransportError::UnsafeStorage),
            "{shape}"
        );
        assert!(fs::symlink_metadata(&path).is_ok(), "{shape}：残留原样");
        assert_eq!(store.bytes(), old, "{shape}");
        assert_eq!(
            TransportRebind::open_existing(&store.approval).err(),
            Some(TransportError::UnsafeStorage),
            "{shape}"
        );
        assert!(fs::symlink_metadata(&path).is_ok(), "{shape}：残留原样");
    }
}

#[test]
fn a_missing_corrupt_or_unapproved_record_keeps_the_residue_in_place() {
    for (shape, expected) in [
        ("missing", TransportError::MissingStore),
        ("corrupt", TransportError::CorruptStore),
        ("binding", TransportError::BindingMismatch),
        ("roots", TransportError::RootsChanged),
        ("stale", TransportError::ConfigurationChanged),
    ] {
        let store = Store::new();
        leave(&store, b"uncommitted");
        let record = root(&store).join(paths::STATE);
        match shape {
            "missing" => fs::remove_file(&record).expect("可以移走"),
            "corrupt" => write_private(&record, b"{}"),
            "binding" => store.approval.binding.borrow_mut().scope_hash = [9; 32],
            "roots" => store.approval.roots.borrow_mut().mirror_identity = (1, 1),
            _ => {}
        }
        let before = fs::read(&record).ok();
        let launcher = FakeLauncher::start();
        let backend =
            verify_backend_at(&store.approval, launcher.socket()).expect("假 launcher 探测通过");
        let runtime = PinnedRuntime::from_approval(&store.approval).expect("固定运行时可取");
        if shape == "stale" {
            *store.approval.fresh.borrow_mut() = false;
        }
        assert_eq!(
            SyncTransportOwner::open_existing(&store.approval, runtime, backend).err(),
            Some(expected),
            "{shape}"
        );
        assert_eq!(
            fs::read(residue(&store)).expect("残留原样"),
            b"uncommitted",
            "{shape}"
        );
        assert_eq!(fs::read(&record).ok(), before, "{shape}");
    }
}

#[test]
fn another_owner_holding_the_lease_keeps_the_residue_in_place() {
    let store = Store::new();
    let held = FilePublisher::open_existing(&root(&store)).expect("可以先占住租约");
    leave(&store, b"uncommitted");
    assert_eq!(open(&store).err(), Some(TransportError::Busy));
    assert_eq!(
        TransportRebind::open_existing(&store.approval).err(),
        Some(TransportError::Busy)
    );
    assert_eq!(fs::read(residue(&store)).expect("残留原样"), b"uncommitted");
    drop(held);
    open(&store).expect("租约放开之后照常");
    assert!(!residue(&store).exists());
}

#[test]
fn the_offline_rebind_leaves_the_residue_alone_until_it_is_told_to_discard_it() {
    // 自己的拒绝项没过：不动。
    let store = Store::new();
    leave(&store, b"uncommitted");
    store.approval.binding.borrow_mut().scope_hash = [9; 32];
    assert_eq!(
        TransportRebind::open_existing(&store.approval).err(),
        Some(TransportError::BindingMismatch)
    );
    assert_eq!(fs::read(residue(&store)).expect("残留原样"), b"uncommitted");

    // 打开是只读的；没让丢就提交，发布照旧撞上残留失败关闭。
    let store = Store::new();
    leave(&store, b"uncommitted");
    let old = store.bytes();
    let pending = TransportRebind::open_existing(&store.approval).expect("残留不挡打开");
    assert!(pending.has_residue());
    assert_eq!(fs::read(residue(&store)).expect("残留原样"), b"uncommitted");
    assert_eq!(pending.commit(), Err(TransportError::TemporaryResidue));
    assert_eq!(fs::read(residue(&store)).expect("残留原样"), b"uncommitted");
    assert_eq!(store.bytes(), old);

    // 丢之前再复验一遍：审批已经漂移就不丢。
    let mut pending = TransportRebind::open_existing(&store.approval).expect("可以重开");
    *store.approval.fresh.borrow_mut() = false;
    assert_eq!(
        pending.discard_residue(),
        Err(TransportError::ConfigurationChanged)
    );
    assert_eq!(fs::read(residue(&store)).expect("残留原样"), b"uncommitted");
    *store.approval.fresh.borrow_mut() = true;

    assert_eq!(pending.discard_residue(), Ok(true));
    assert!(!pending.has_residue());
    assert!(!residue(&store).exists());
    assert_eq!(store.bytes(), old);
    // 再来一次没有东西可丢，也不动磁盘。
    assert_eq!(pending.discard_residue(), Ok(false));
    pending.commit().expect("可以照常重绑定");
}
