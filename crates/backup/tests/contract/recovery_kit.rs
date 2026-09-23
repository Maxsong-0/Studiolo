//! 恢复检查契约：前置检查全过才恢复主人与密钥映射；Auth 恢复必须让旧会话/L3/恢复码失效
//! 并要求新的签发代次；随同恢复的 AgentStore 旧凭据失效、策略库吊销核对后才可启用。
//!
//! 本文件管恢复检查与启用门禁；kit 兑换与 transcript 在 `bootstrap`。

mod bootstrap;

use studiolo_backup::recovery::kit::{
    AuthRestoreInvalidation, BidirectionalSyncGate, CutoverState, KitError, PreRestoreChecks,
};

fn invalidation() -> AuthRestoreInvalidation {
    AuthRestoreInvalidation {
        old_sessions_invalidated: true,
        l3_invalidated: true,
        snapshot_recovery_codes_blocked: true,
        consumed_records_preserved: true,
        audit_anchor_preserved: true,
        owner_binding_preserved: true,
        reauthenticated: true,
        issuance_generation: 8,
        agent_store_included: true,
        agent_type_tokens_invalidated: true,
        agent_instances_invalidated: true,
        policy_included: true,
        policy_revocations_reconciled: true,
    }
}

fn open_gate() -> BidirectionalSyncGate {
    BidirectionalSyncGate {
        divergence_isolated: true,
        manually_reconciled: true,
        new_checkpoint_accepted: true,
        write_and_backup_chain_reaccepted: true,
        old_root_retained: true,
        policy_included: true,
        policy_revocations_reconciled: true,
    }
}

#[test]
fn every_pre_restore_check_must_pass_with_sync_and_execution_adapters_disabled() {
    let complete = PreRestoreChecks {
        sync_adapter_disabled: true,
        execution_adapter_disabled: true,
        content_verified: true,
        version_verified: true,
        permissions_verified: true,
        hashes_verified: true,
        audit_continuity_verified: true,
        space_verified: true,
    };
    assert!(complete.is_complete());

    let mut sync_still_on = complete;
    sync_still_on.sync_adapter_disabled = false;
    assert!(!sync_still_on.is_complete());

    let mut no_space_check = complete;
    no_space_check.space_verified = false;
    assert!(!no_space_check.is_complete());
}

#[test]
fn auth_restore_must_invalidate_old_sessions_l3_and_snapshot_recovery_codes() {
    invalidation().check(7).expect("完整失效");

    let mut sessions = invalidation();
    sessions.old_sessions_invalidated = false;
    assert_eq!(
        sessions.check(7).err(),
        Some(KitError::OldCredentialsStillValid)
    );

    let mut codes = invalidation();
    codes.snapshot_recovery_codes_blocked = false;
    assert_eq!(
        codes.check(7).err(),
        Some(KitError::SnapshotRecoveryCodeReusable)
    );
}

#[test]
fn dropping_consumed_records_or_the_audit_anchor_is_not_an_acceptable_shortcut() {
    let mut dropped = invalidation();
    dropped.consumed_records_preserved = false;
    assert_eq!(
        dropped.check(7).err(),
        Some(KitError::ConsumedRecordsDropped)
    );

    let mut anchor = invalidation();
    anchor.audit_anchor_preserved = false;
    assert_eq!(
        anchor.check(7).err(),
        Some(KitError::ConsumedRecordsDropped)
    );
}

#[test]
fn an_old_snapshot_restore_needs_a_new_issuance_generation() {
    let mut same = invalidation();
    same.issuance_generation = 7;
    assert_eq!(
        same.check(7).err(),
        Some(KitError::IssuanceGenerationNotAdvanced),
        "代次不前进就可能复用旧 event_id"
    );
    assert!(invalidation().check(7).is_ok());
}

#[test]
fn an_included_agent_store_invalidates_its_old_type_tokens_and_instances() {
    let mut tokens = invalidation();
    tokens.agent_type_tokens_invalidated = false;
    assert_eq!(
        tokens.check(7).err(),
        Some(KitError::AgentStoreCredentialsStillValid)
    );

    let mut not_included = invalidation();
    not_included.agent_store_included = false;
    not_included.agent_type_tokens_invalidated = false;
    not_included.agent_instances_invalidated = false;
    assert!(
        not_included.check(7).is_ok(),
        "未纳入的 AgentStore 不作要求"
    );
}

#[test]
fn the_owner_binding_is_kept_and_only_a_reauthentication_allows_the_final_cutover() {
    let mut not_reauthenticated = invalidation();
    not_reauthenticated.reauthenticated = false;
    assert_eq!(
        not_reauthenticated.check(7).err(),
        Some(KitError::OwnerBindingNotReauthenticated)
    );
    assert_eq!(
        serde_json::to_value(CutoverState::IntentRecorded).expect("encode"),
        serde_json::json!("intent_recorded")
    );
}

#[test]
fn bidirectional_sync_stays_off_until_every_gate_is_satisfied() {
    let open = open_gate();
    assert!(open.may_enable());

    for mutate in [
        |gate: &mut BidirectionalSyncGate| gate.divergence_isolated = false,
        |gate: &mut BidirectionalSyncGate| gate.manually_reconciled = false,
        |gate: &mut BidirectionalSyncGate| gate.new_checkpoint_accepted = false,
        |gate: &mut BidirectionalSyncGate| gate.write_and_backup_chain_reaccepted = false,
        |gate: &mut BidirectionalSyncGate| gate.old_root_retained = false,
        |gate: &mut BidirectionalSyncGate| gate.policy_revocations_reconciled = false,
    ] {
        let mut gate = open;
        mutate(&mut gate);
        assert!(!gate.may_enable());
    }
}

#[test]
fn an_included_policy_store_fails_the_restore_check_until_its_revocations_are_reconciled() {
    invalidation().check(7).expect("吊销已核对");

    let mut unreconciled = invalidation();
    unreconciled.policy_revocations_reconciled = false;
    assert_eq!(
        unreconciled.check(7).err(),
        Some(KitError::PolicyRevocationsNotReconciled)
    );
    assert_eq!(
        KitError::PolicyRevocationsNotReconciled.to_string(),
        "策略库恢复后未核对吊销：旧快照可能让已吊销的实例或类型复活"
    );

    let mut not_included = invalidation();
    not_included.policy_included = false;
    not_included.policy_revocations_reconciled = false;
    assert!(not_included.check(7).is_ok(), "未纳入的策略库不作要求");
}

#[test]
fn bidirectional_sync_stays_off_while_a_restored_policy_store_has_unreconciled_revocations() {
    let mut gate = open_gate();
    gate.policy_revocations_reconciled = false;
    assert!(!gate.may_enable(), "纳入策略库而未核对吊销时不得启用");

    gate.policy_included = false;
    assert!(gate.may_enable(), "未纳入策略库时不作要求");
}

#[test]
fn a_restore_record_that_omits_a_policy_fact_fails_closed_instead_of_defaulting() {
    // 完整记录能往返，缺字段的失败才归因于缺字段本身。
    let invalidation_record = serde_json::to_value(invalidation()).expect("encode");
    let gate_record = serde_json::to_value(open_gate()).expect("encode");
    assert_eq!(
        serde_json::from_value::<AuthRestoreInvalidation>(invalidation_record.clone())
            .expect("decode"),
        invalidation()
    );
    assert_eq!(
        serde_json::from_value::<BidirectionalSyncGate>(gate_record.clone()).expect("decode"),
        open_gate()
    );

    for field in ["policy_included", "policy_revocations_reconciled"] {
        let mut record = invalidation_record.clone();
        record.as_object_mut().expect("object").remove(field);
        assert!(
            serde_json::from_value::<AuthRestoreInvalidation>(record).is_err(),
            "恢复检查记录缺 {field} 不能被默认补齐"
        );

        let mut record = gate_record.clone();
        record.as_object_mut().expect("object").remove(field);
        assert!(
            serde_json::from_value::<BidirectionalSyncGate>(record).is_err(),
            "启用门禁记录缺 {field} 不能被默认补齐"
        );
    }
}
