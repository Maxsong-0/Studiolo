//! 失败与恢复契约：任何中途失败都只留下隔离且不能服务的新目录；
//! 身份、逻辑引用与半初始化残留在动第一个字节之前就被拒绝。

use super::super::{LineageState, RestoreError, RestoreFault};
use super::Fixture;
use crate::{BrokerError, FsVaultBroker};
use std::fs;

const INTERRUPTION_POINTS: [RestoreFault; 5] = [
    RestoreFault::AfterLineage,
    RestoreFault::AfterHistory,
    RestoreFault::AfterMaintenance,
    RestoreFault::AfterDataBinding,
    RestoreFault::AfterCanonicalBinding,
];

#[test]
fn every_interruption_point_leaves_a_new_directory_that_cannot_serve() {
    for point in INTERRUPTION_POINTS {
        let fixture = Fixture::restored();
        let error = fixture.rebind_with_fault(point).unwrap_err();
        assert!(
            matches!(error, RestoreError::InterruptedByFault),
            "{point:?}"
        );
        if point == RestoreFault::AfterCanonicalBinding {
            // 这一点之后配对已经一致，但 lineage 仍是 Rebinding，必须显式接手。
            let lineage = super::super::read_restore_lineage(&fixture.new_data)
                .unwrap()
                .unwrap();
            assert_eq!(lineage.state, LineageState::Rebinding);
            assert!(!lineage.is_completed());
            assert!(matches!(fixture.rebind(), Err(RestoreError::Interrupted)));
        } else {
            assert!(
                matches!(fixture.open_new(), Err(BrokerError::WriterBindingMismatch)),
                "{point:?} 之后新目录必须仍然不可服务"
            );
        }
    }
}

#[test]
fn an_interrupted_rebind_is_only_resumed_when_the_operator_says_so() {
    let fixture = Fixture::restored();
    fixture
        .rebind_with_fault(RestoreFault::AfterDataBinding)
        .unwrap_err();

    assert!(matches!(fixture.rebind(), Err(RestoreError::Interrupted)));

    let mut request = fixture.request();
    request.resume_interrupted = true;
    let receipt = super::super::rebind_new_roots(&request).unwrap();
    assert_eq!(receipt.lineage.state, LineageState::Completed);
    assert!(fixture.open_new().is_ok());
}

#[test]
fn a_second_writer_on_the_new_canonical_root_blocks_the_rebind() {
    use rustix::fs::{FlockOperation, flock};
    let fixture = Fixture::restored();
    let held = fs::File::open(&fixture.new_vault).unwrap();
    flock(&held, FlockOperation::NonBlockingLockExclusive).unwrap();

    assert!(matches!(fixture.rebind(), Err(RestoreError::NewRootBusy)));
    assert!(
        super::super::read_restore_lineage(&fixture.new_data)
            .unwrap()
            .is_none(),
        "被占用时一个字节都不该写"
    );

    flock(&held, FlockOperation::Unlock).unwrap();
    assert!(fixture.rebind().is_ok());
}

#[test]
fn a_wrong_pair_id_is_refused_before_any_byte_is_written() {
    let fixture = Fixture::restored();
    let mut request = fixture.request();
    request.expected_pair_id = crate::VaultUuid::new();

    assert!(matches!(
        super::super::rebind_new_roots(&request),
        Err(RestoreError::PairIdMismatch)
    ));
    assert!(
        super::super::read_restore_lineage(&fixture.new_data)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_wrong_vault_identity_is_refused() {
    let fixture = Fixture::restored();
    let mut request = fixture.request();
    request.expected_vault_uuid = crate::VaultUuid::new();

    assert!(matches!(
        super::super::rebind_new_roots(&request),
        Err(RestoreError::VaultIdentityMismatch)
    ));
}

#[test]
fn diverged_broker_bindings_are_refused_instead_of_being_silently_reconciled() {
    let fixture = Fixture::restored();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.new_data.join("broker-binding.json")).unwrap())
            .unwrap();
    value["snapshots"]["ino"] = serde_json::json!(999_999_u64);
    fs::write(
        fixture.new_data.join("broker-binding.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();

    assert!(matches!(
        fixture.rebind(),
        Err(RestoreError::BindingsDiverged)
    ));
}

#[test]
fn a_logical_reference_pointing_at_another_pair_fails_the_whole_rebind() {
    let fixture = Fixture::restored();
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(fixture.new_data.join("writer-state.json")).unwrap())
            .unwrap();
    value["pair_id"] = serde_json::json!(crate::VaultUuid::new().to_string());
    fs::write(
        fixture.new_data.join("writer-state.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();

    assert!(matches!(
        fixture.rebind(),
        Err(RestoreError::LogicalReferenceMismatch {
            reference: "writer-state.json"
        })
    ));
}

#[test]
fn a_history_metadata_group_bound_to_another_pair_fails_the_rebind() {
    let fixture = Fixture::restored();
    let path = fixture.new_data.join("history/meta-v1/binding.json");
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["pair_id"] = serde_json::json!(crate::VaultUuid::new().to_string());
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

    assert!(matches!(
        fixture.rebind(),
        Err(RestoreError::PairIdMismatch)
    ));
}

#[test]
fn a_half_initialized_maintenance_directory_refuses_the_rebind() {
    let fixture = Fixture::restored();
    fs::create_dir(fixture.new_data.join("maintenance-v1.preparing")).unwrap();

    assert!(matches!(
        fixture.rebind(),
        Err(RestoreError::MaintenanceHalfInitialized)
    ));
}

#[test]
fn an_ordinary_initialize_never_repairs_a_copied_pair() {
    let fixture = Fixture::restored();
    // `open` 走的是「显式初始化或采用旧存储」，它同样不许自动修复新物理绑定。
    assert!(matches!(
        FsVaultBroker::open(&fixture.new_vault, &fixture.new_data),
        Err(BrokerError::WriterBindingMismatch)
    ));
    assert!(
        super::super::read_restore_lineage(&fixture.new_data)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_missing_new_data_root_is_reported_instead_of_being_created() {
    let fixture = Fixture::restored();
    fs::remove_dir_all(fixture.new_data.join("snapshots")).unwrap();

    assert!(matches!(
        fixture.rebind(),
        Err(RestoreError::NewRootIncomplete)
    ));
}

#[test]
fn an_empty_or_control_character_checkpoint_id_is_rejected() {
    let fixture = Fixture::restored();
    for bad in ["", "   ", "cp\u{0}1"] {
        let mut request = fixture.request();
        request.checkpoint_id = bad.to_owned();
        assert!(matches!(
            super::super::rebind_new_roots(&request),
            Err(RestoreError::InvalidCheckpointId)
        ));
    }
}
