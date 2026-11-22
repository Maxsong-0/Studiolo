//! BUG-046 接缝：写者没开文件操作时，经接缝删除确定「没尝试」，写者数据一个字节都不碰；
//! 按原键重放同样「没尝试」，首次执行和重开核对都据此核成无效果，批次才能闭合。
//! 开了文件操作则照常走文件操作协议，同一原键重放拿回同一张票据。

use super::fixture::{Fixture, path};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use studiolo_local_server::sync_plane::driver::content::{
    BridgeSeamError, canonical_deletes_enabled, delete_canonical,
};
use studiolo_local_server::sync_plane::driver::reconcile::replay_outcome;
use studiolo_local_server::sync_plane::job_backend::{SeamVerdict, first_attempt_outcome};
use studiolo_sync_bridge::Binding;
use studiolo_sync_bridge::executor::{JobOutcome, NoEffectCause};
use studiolo_vault_broker::FileOperationOutcome;

const KEY: &str = "bridge/plan-11/delete-original-key";

fn binding() -> Binding {
    Binding {
        link_id: [0x11; 16],
        canonical_vault: [0x22; 16],
        remote_vault: [0x33; 16],
        canonical_root_token: [0x44; 32],
        mirror_root_token: [0x55; 32],
        scope_hash: [0x66; 32],
        scope_epoch: 1,
    }
}

/// 目录下每个普通文件的相对路径与内容。
fn tree(root: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut pending = vec![root.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let entry = entry.unwrap();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                let name = entry.path().strip_prefix(root).unwrap().display().to_string();
                files.insert(name, fs::read(entry.path()).unwrap());
            }
        }
    }
    files
}

#[tokio::test]
async fn deleting_without_file_operations_is_not_attempted_and_touches_nothing() {
    let f = Fixture::new(b"mirror bytes");
    let owner = f.writer().await;
    assert_eq!(canonical_deletes_enabled(&owner), Ok(false));
    let before = tree(&f.root.join("data"));

    for attempt in ["首次执行", "按原键重放"] {
        let deleted = delete_canonical(&owner, &path(), f.previous, KEY, &binding());
        assert_eq!(deleted.err(), Some(BridgeSeamError::NotAttempted), "{attempt}");
    }

    assert_eq!(
        fs::read(f.root.join("vault/notes/body.txt")).unwrap(),
        b"canonical before bridge"
    );
    assert_eq!(tree(&f.root.join("data")), before, "写者数据一个字节都没动");
    let no_effect = JobOutcome::NoEffect {
        reason: NoEffectCause::CancelledBeforeInstall,
        terminal_id: format!("not-attempted:{KEY}"),
    };
    assert_eq!(first_attempt_outcome(SeamVerdict::NotAttempted, KEY), no_effect);
    assert_eq!(replay_outcome(SeamVerdict::NotAttempted, true, KEY), no_effect);
    owner.shutdown().await.unwrap();
}

#[tokio::test]
async fn deleting_with_file_operations_goes_through_the_protocol() {
    let f = Fixture::new(b"mirror bytes");
    let owner = f.writer_with_file_operations().await;
    assert_eq!(canonical_deletes_enabled(&owner), Ok(true));

    let receipt = delete_canonical(&owner, &path(), f.previous, KEY, &binding()).unwrap();

    assert_eq!(receipt.outcome, FileOperationOutcome::Committed);
    assert!(!f.root.join("vault/notes/body.txt").exists());
    let replayed = delete_canonical(&owner, &path(), f.previous, KEY, &binding()).unwrap();
    assert_eq!(replayed, receipt, "同一原键重放不第二次删除");
    owner.shutdown().await.unwrap();
}
