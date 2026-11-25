//! 平面与 Bridge 未决事实不能靠重绑定抹掉；同一排他租约仍是维护前置。

use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, sync},
    sync::Sync,
    sync_rebind::{edit_approval, initialized, options, run_with, state_bytes},
};
use crate::sync_plane::{bridge_binding, bridge_limits, recovery::PlaneStateStore};
use serde_json::json;
use std::{fs, os::unix::fs::PermissionsExt, path::PathBuf};
use studiolo_sync_bridge::{ApprovedScope, Input, Inventory, Side, sqlite::BridgeState};
use studiolo_sync_transport::{owned_run::TransportError, store::FilePublisher};

#[test]
fn started_but_unreaped_plane_sequence_blocks_rebind() {
    let s = initialized();
    let old = state_bytes(&s);
    let mut plane = PlaneStateStore::open(&s.root().join("plane")).unwrap();
    let mut pending = plane.state().clone();
    pending.started_sequence = Some(1);
    plane.save(pending).unwrap();
    drop(plane);
    assert_eq!(
        s.run(Command::SyncRebind, None),
        Err(MaintenanceError::SyncRebindPending)
    );
    assert_eq!(state_bytes(&s), old);
}

#[test]
fn unresolved_bridge_batch_blocks_rebind_even_when_transport_is_idle() {
    let s = initialized();
    let old = state_bytes(&s);
    {
        let options = options(&s);
        let inputs = Inputs::open(&options).unwrap();
        let scope = sync::scope(&inputs).unwrap();
        let approved = sync::approve(&scope, &inputs, &options).unwrap();
        let binding = bridge_binding(&approved, inputs.roots.vault_identity());
        let mut bridge =
            BridgeState::open_existing(&s.root().join("plane/bridge"), binding.clone()).unwrap();
        let view = bridge.view().unwrap();
        let inventory = |side| Inventory {
            binding: binding.clone(),
            side,
            sequence: 1,
            complete: true,
            entries: Default::default(),
        };
        bridge
            .reserve(
                view.version,
                Input {
                    scope: ApprovedScope {
                        binding: binding.clone(),
                        paths: Default::default(),
                    },
                    canonical: inventory(Side::Canonical),
                    mirror: inventory(Side::Mirror),
                    expected_baseline_version: view.cursor.baseline_version,
                    plan_sequence: 1,
                    cursor: view.cursor,
                    baseline: view.baseline,
                    limits: bridge_limits(&scope),
                },
            )
            .unwrap();
        assert!(bridge.view().unwrap().pending_sequence.is_some());
    }
    assert_eq!(
        s.run(Command::SyncRebind, None),
        Err(MaintenanceError::SyncRebindPending)
    );
    assert_eq!(state_bytes(&s), old);
}

#[test]
fn existing_plane_or_transport_owner_prevents_rebind_and_preserves_bytes() {
    let s = initialized();
    let old = state_bytes(&s);
    let plane = PlaneStateStore::open(&s.root().join("plane")).unwrap();
    assert_eq!(
        s.run(Command::SyncRebind, None),
        Err(MaintenanceError::SyncStorage)
    );
    drop(plane);
    let _transport = FilePublisher::open_existing(&s.root().join("transport")).unwrap();
    assert!(s.run(Command::SyncRebind, None).is_err());
    assert_eq!(state_bytes(&s), old);
}

fn leave_publish(s: &Sync, mode: u32) -> PathBuf {
    let residue = s.root().join("transport/state.json.publish");
    fs::write(&residue, "uncommitted").unwrap();
    fs::set_permissions(&residue, fs::Permissions::from_mode(mode)).unwrap();
    residue
}

#[test]
fn a_leftover_publish_of_an_unexpected_shape_is_refused_before_the_review() {
    let s = initialized();
    let old = state_bytes(&s);
    let residue = leave_publish(&s, 0o644);
    assert_eq!(
        run_with(&s, |_, _| panic!("形态不合规不该走到确认")),
        Err(MaintenanceError::SyncRebind(TransportError::UnsafeStorage))
    );
    assert_eq!(fs::read_to_string(&residue).unwrap(), "uncommitted");
    assert_eq!(state_bytes(&s), old);
}

#[test]
fn a_leftover_publish_survives_every_refusal_before_the_confirmed_commit() {
    // 平面还有没回收的运行：传输槽自己的拒绝项都过了，也不许先动残留。
    let s = initialized();
    let old = state_bytes(&s);
    let residue = leave_publish(&s, 0o600);
    let mut plane = PlaneStateStore::open(&s.root().join("plane")).unwrap();
    let mut pending = plane.state().clone();
    pending.started_sequence = Some(1);
    plane.save(pending).unwrap();
    drop(plane);
    assert_eq!(
        run_with(&s, |_, _| panic!("平面未决不该走到确认")),
        Err(MaintenanceError::SyncRebindPending)
    );
    assert_eq!(fs::read_to_string(&residue).unwrap(), "uncommitted");
    assert_eq!(state_bytes(&s), old);

    // 人在确认处看到有残留要丢，拒绝之后两个文件一个字节不动。
    let s = initialized();
    let old = state_bytes(&s);
    let residue = leave_publish(&s, 0o600);
    assert_eq!(
        run_with(&s, |review, _| {
            assert_eq!(review["discards_publish_residue"], true);
            Err(MaintenanceError::Confirmation)
        }),
        Err(MaintenanceError::Confirmation)
    );
    assert_eq!(fs::read_to_string(&residue).unwrap(), "uncommitted");
    assert_eq!(state_bytes(&s), old);

    // 确认期间审批换了：复核不过，也不动。
    let s = initialized();
    let old = state_bytes(&s);
    let residue = leave_publish(&s, 0o600);
    assert_eq!(
        run_with(&s, |_, _| {
            edit_approval(&s, |doc| doc["budget"]["deadline_seconds"] = json!(1500));
            Ok(())
        }),
        Err(MaintenanceError::Configuration)
    );
    assert_eq!(fs::read_to_string(&residue).unwrap(), "uncommitted");
    assert_eq!(state_bytes(&s), old);
}

#[test]
fn a_confirmed_rebind_discards_the_leftover_publish_and_commits() {
    let s = initialized();
    let old = state_bytes(&s);
    let residue = leave_publish(&s, 0o600);
    run_with(&s, |review, _| {
        assert_eq!(review["discards_publish_residue"], true);
        assert!(residue.exists(), "确认之前不丢");
        Ok(())
    })
    .unwrap();
    assert!(!residue.exists());
    assert_ne!(state_bytes(&s), old);
    // 没有残留时确认页也如实写。
    run_with(&s, |review, _| {
        assert_eq!(review["discards_publish_residue"], false);
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_failed_discard_is_reported_as_not_committed_and_publishes_nothing() {
    let s = initialized();
    let old = state_bytes(&s);
    let residue = leave_publish(&s, 0o600);
    let opened = residue.with_file_name("opened-earlier");
    // 确认期间那个名字换成了别的文件：不是打开时的那一个，不碰，也还没发布。
    assert_eq!(
        run_with(&s, |_, _| {
            fs::rename(&residue, &opened).unwrap();
            leave_publish(&s, 0o600);
            Ok(())
        }),
        Err(MaintenanceError::SyncRebind(TransportError::UnsafeStorage))
    );
    assert_eq!(fs::read_to_string(&residue).unwrap(), "uncommitted");
    assert_eq!(fs::read_to_string(&opened).unwrap(), "uncommitted");
    assert_eq!(state_bytes(&s), old);
}
