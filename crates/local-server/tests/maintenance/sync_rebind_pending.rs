//! 平面与 Bridge 未决事实不能靠重绑定抹掉；同一排他租约仍是维护前置。

use super::{
    super::{MaintenanceError, inputs::Inputs, options::Command, sync},
    sync_rebind::{initialized, options, state_bytes},
};
use crate::sync_plane::{bridge_binding, bridge_limits, recovery::PlaneStateStore};
use studiolo_sync_bridge::{ApprovedScope, Input, Inventory, Side, sqlite::BridgeState};
use studiolo_sync_transport::store::FilePublisher;

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
