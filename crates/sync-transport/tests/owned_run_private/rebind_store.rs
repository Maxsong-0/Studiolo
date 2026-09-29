//! 真实传输文件租约、同实体改名与发布故障；不需要执行旧软件或启动 launcher。

use super::support::FakeApproval;
use crate::owned_run::*;
use crate::store::{FilePublisher, StatePublisher, publish::PublishFault};
use std::{
    fs,
    os::unix::fs::{MetadataExt, PermissionsExt},
};

struct Store {
    _temp: tempfile::TempDir,
    approval: FakeApproval,
}

impl Store {
    fn new() -> Self {
        let temp = tempfile::tempdir_in(fs::canonicalize("/tmp").unwrap()).unwrap();
        let private = temp.path().join("transport");
        let mirror = temp.path().join("mirror-view");
        for path in [&private, &mirror] {
            fs::create_dir(path).unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let approval = FakeApproval::new();
        {
            let mut roots = approval.roots.borrow_mut();
            let p = fs::metadata(&private).unwrap();
            let m = fs::metadata(&mirror).unwrap();
            roots.private_root = private;
            roots.private_identity = (p.dev(), p.ino());
            roots.mirror_view_root = mirror;
            roots.mirror_identity = (m.dev(), m.ino());
        }
        let runtime = PinnedRuntime::from_approval(&approval).unwrap();
        initialize_store(&approval, &runtime).unwrap();
        Self {
            _temp: temp,
            approval,
        }
    }

    fn bytes(&self) -> Vec<u8> {
        fs::read(self.approval.roots().private_root.join("state.json")).unwrap()
    }
}

#[test]
fn maintenance_holds_the_existing_lease_until_commit_or_cancellation() {
    let store = Store::new();
    let pending = TransportRebind::open_existing(&store.approval).unwrap();
    assert_eq!(
        TransportRebind::open_existing(&store.approval).err(),
        Some(TransportError::Busy)
    );
    let old = store.bytes();
    drop(pending);
    assert_eq!(store.bytes(), old);
    TransportRebind::open_existing(&store.approval)
        .unwrap()
        .commit()
        .unwrap();
    assert_eq!(state::decode(&store.bytes()).unwrap().generation, 2);
}

#[test]
fn moved_same_inode_store_reopens_at_new_path_without_old_path_or_software() {
    let store = Store::new();
    let old = state::decode(&store.bytes()).unwrap();
    let mut roots = store.approval.roots.borrow_mut();
    let moved = roots.private_root.with_file_name("renamed-transport");
    fs::rename(&roots.private_root, &moved).unwrap();
    roots.private_root = moved;
    drop(roots);
    store.approval.binding.borrow_mut().approved_config_digest = [8; 32];
    TransportRebind::open_existing(&store.approval)
        .unwrap()
        .commit()
        .unwrap();
    let new = state::decode(&store.bytes()).unwrap();
    assert_eq!(new.installation_id, old.installation_id);
    assert_eq!(new.next_sequence, old.next_sequence);
    assert_eq!(new.roots.private_identity, old.roots.private_identity);
    assert_ne!(new.roots.private_root, old.roots.private_root);
}

struct Faulted(FilePublisher, PublishFault);

impl StatePublisher for Faulted {
    fn read(&self) -> Result<Vec<u8>> {
        self.0.read()
    }
    fn check(&self) -> Result<()> {
        self.0.check()
    }
    fn owner_identity(&self) -> (u32, u32) {
        self.0.owner_identity()
    }
    fn replace(&mut self, bytes: &[u8]) -> Result<()> {
        self.0.replace_faulted(bytes, self.1)
    }
}

#[test]
fn short_write_or_sync_failure_leaves_authoritative_state_old_and_residue_untrusted() {
    for fault in [PublishFault::ShortWrite, PublishFault::SyncFailure] {
        let store = Store::new();
        let old = store.bytes();
        let publisher = FilePublisher::open_existing(&store.approval.roots().private_root).unwrap();
        let pending =
            TransportRebind::open(&store.approval, Box::new(Faulted(publisher, fault))).unwrap();
        assert_eq!(pending.commit(), Err(TransportError::Unavailable));
        assert_eq!(store.bytes(), old);
        assert_eq!(
            TransportRebind::open_existing(&store.approval).err(),
            Some(TransportError::TemporaryResidue)
        );
    }
}

#[test]
fn an_empty_private_root_is_not_initialized_by_rebind() {
    let store = Store::new();
    let empty = store._temp.path().join("empty");
    fs::create_dir(&empty).unwrap();
    fs::set_permissions(&empty, fs::Permissions::from_mode(0o700)).unwrap();
    store.approval.roots.borrow_mut().private_root = empty.clone();
    assert_eq!(
        TransportRebind::open_existing(&store.approval).err(),
        Some(TransportError::MissingStore)
    );
    assert_eq!(fs::read_dir(empty).unwrap().count(), 0);
}
