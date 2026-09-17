use super::*;
#[test]
fn enabled_missing_or_replaced_slot_is_not_treated_as_legacy_idle() {
    for entire in [false, true] {
        let fixture = Fixture::new();
        let (mut broker, _) = fixture.pending(1);
        let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
        intent(&mut broker, &ticket);
        drop(broker);
        if entire {
            fs::remove_dir_all(fixture.data.join("maintenance-v1")).unwrap();
        } else {
            fs::remove_file(fixture.data.join("maintenance-v1/slot.json")).unwrap();
        }
        assert!(FsVaultBroker::open_existing(&fixture.root, &fixture.data).is_err());
        assert_eq!(fixture.body(), b"old");
    }
}
#[test]
fn same_instance_slot_inode_replacement_and_bound_root_changes_fail_closed() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    let slot = fixture.data.join("maintenance-v1/slot.json");
    let copied = fixture.data.join("maintenance-v1/replacement.json");
    fs::write(&copied, fs::read(&slot).unwrap()).unwrap();
    fs::rename(copied, slot).unwrap();
    assert!(
        broker
            .confirm_maintenance_intent(&ticket, 1, &"a".repeat(64))
            .is_err()
    );
    drop(broker);
    let mut broker = fixture.reopen();
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    fs::rename(&fixture.root, fixture.root.with_extension("moved")).unwrap();
    fs::create_dir(&fixture.root).unwrap();
    assert!(
        broker
            .confirm_maintenance_intent(&ticket, 1, &"a".repeat(64))
            .is_err()
    );
}
#[test]
fn legacy_v1_pending_can_cancel_without_inventing_a_recorded_origin() {
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let mut state = crate::mutation::load(&broker).unwrap();
    state.format_version = 1;
    state.operation.as_mut().unwrap().recording = None;
    crate::mutation::save(&broker, &state).unwrap();
    let ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    intent(&mut broker, &ticket);
    broker.commit_maintenance(ticket).unwrap();
    flush(&mut broker);
    let state = crate::mutation::load(&broker).unwrap();
    assert_eq!(state.format_version, 3);
    assert!(state.maintenance_enabled);
    assert_eq!(state.committed_through, 1);
    assert_eq!(fixture.body(), b"old");
}
#[test]
fn partial_schema_and_unknown_slot_version_are_never_implicitly_repaired() {
    let fixture = Fixture::new();
    let broker = fixture.open();
    drop(broker);
    fs::create_dir(fixture.data.join("maintenance-v1.preparing")).unwrap();
    assert!(FsVaultBroker::open_existing(&fixture.root, &fixture.data).is_err());
    let fixture = Fixture::new();
    let (mut broker, _) = fixture.pending(1);
    let _ticket = prepare(&mut broker, MaintenanceDecision::CancelPrepared);
    drop(broker);
    let path = fixture.data.join("maintenance-v1/slot.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["version"] = 9.into();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(FsVaultBroker::open_existing(&fixture.root, &fixture.data).is_err());
    assert_eq!(fixture.body(), b"old");
}
