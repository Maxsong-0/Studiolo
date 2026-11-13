use super::*;
use std::{fs, os::unix::fs::PermissionsExt};
use studiolo_audit_ledger::SqliteAuditLedger;
use studiolo_core::checkpoint::{CheckpointId, FreezeBudget};
use studiolo_policy::PolicyStore;

#[test]
fn actual_plane_permits_are_drained_and_released_on_success_and_failure() {
    let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let paths: Vec<_> = ["policy", "audit", "copy"]
        .map(|name| temp.path().join(name))
        .into();
    for path in &paths {
        fs::create_dir(path).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    let plane = crate::PolicyPlane::new(
        PolicyStore::open(&paths[0]).unwrap(),
        Arc::new(SqliteAuditLedger::open(&paths[1]).unwrap()),
    );
    let participant = plane.checkpoint(paths[2].clone()).unwrap();
    let inner = participant.inner.as_ref().unwrap();
    let request = FreezeRequest {
        checkpoint_id: CheckpointId::parse("policy-test").unwrap(),
        budget: FreezeBudget::new(1000).unwrap(),
    };
    let held = inner.permits.clone().try_acquire_owned().unwrap();
    let short = FreezeRequest {
        budget: FreezeBudget::new(30).unwrap(),
        ..request.clone()
    };
    assert!(matches!(
        participant.freeze(&short),
        Err(CheckpointError::FreezeTimeout { .. })
    ));
    assert_eq!(inner.permits.available_permits(), 1);
    drop(held);
    assert!(participant.freeze(&request).unwrap().drain.is_drained());
    assert!(inner.permits.try_acquire().is_err());
    assert!(participant.export(&request).unwrap().is_verified());
    let other = FreezeRequest {
        checkpoint_id: CheckpointId::parse("other").unwrap(),
        ..request.clone()
    };
    assert!(matches!(
        participant.export(&other),
        Err(CheckpointError::NotDrained { .. })
    ));
    participant.thaw().unwrap();
    participant.thaw().unwrap();
    assert_eq!(inner.permits.available_permits(), 2);
    assert!(participant.freeze(&request).unwrap().drain.is_drained());
    assert!(matches!(
        participant.export(&request),
        Err(CheckpointError::Unverified { .. })
    ));
    participant.thaw().unwrap();
    assert_eq!(inner.permits.available_permits(), 2);
}

#[test]
fn absent_owner_never_claims_a_real_freeze_or_export() {
    let absent = PolicyParticipant::absent();
    assert_eq!(absent.assembly(), OwnerAssembly::Absent);
    let request = FreezeRequest {
        checkpoint_id: CheckpointId::parse("absent").unwrap(),
        budget: FreezeBudget::new(100).unwrap(),
    };
    assert!(absent.freeze(&request).is_err());
    assert!(absent.export(&request).is_err());
    absent.thaw().unwrap();
}
