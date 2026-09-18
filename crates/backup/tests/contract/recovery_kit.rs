//! recovery kit 与 transcript 契约：只允许本机 TTY/localhost、一次性、有期限；
//! 任何恢复写入之前先建立带完整性保护且不含秘密的 transcript；
//! Auth 恢复必须让旧会话/L3/恢复码失效，并要求新的签发代次。

use super::support::DIGEST;
use studiolo_backup::checkpoint::manifest::{SecretReference, SecretSource};
use studiolo_backup::recovery::kit::{
    AuthRestoreInvalidation, BidirectionalSyncGate, BootstrapChannel, CutoverState, KitError,
    PreRestoreChecks, RecoveryIdentity, RecoveryKit,
};
use studiolo_backup::recovery::transcript::{RecoveryStep, RecoveryTranscript, TranscriptError};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

fn kit() -> RecoveryKit {
    RecoveryKit {
        checkpoint_id: CheckpointId::parse("cp-0001").expect("checkpoint id"),
        identity: RecoveryIdentity {
            identity_id: "restore-identity-1".to_owned(),
            expires_at: InstantMillis::from_millis(10_000),
            redeemed: false,
        },
        package_digest: DIGEST.to_owned(),
        secret_references: vec![SecretReference {
            logical_name: "owner_key_mapping".to_owned(),
            source: SecretSource::EnvironmentVariable {
                variable: "STUDIOLO_OWNER_KEY_FILE".to_owned(),
            },
        }],
        sync_recovery_material: true,
        stored_outside_ordinary_checkpoint: true,
    }
}

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
    }
}

#[test]
fn a_local_tty_bootstrap_opens_the_kit_and_establishes_the_transcript_first() {
    let mut kit = kit();

    let grant = kit
        .redeem(BootstrapChannel::LocalTty, InstantMillis::from_millis(1))
        .expect("redeem");

    assert_eq!(grant.transcript.entries.len(), 1);
    assert_eq!(grant.transcript.entries[0].step, RecoveryStep::KitOpened);
    grant.transcript.verify().expect("链自洽");
    assert!(kit.identity.redeemed);
}

#[test]
fn a_remote_bootstrap_is_refused_outright() {
    let mut kit = kit();

    assert_eq!(
        kit.redeem(
            BootstrapChannel::RemoteNetwork,
            InstantMillis::from_millis(1)
        )
        .err(),
        Some(KitError::RemoteBootstrapForbidden)
    );
    assert!(!kit.identity.redeemed, "被拒绝的兑换不能消耗身份");
}

#[test]
fn the_recovery_identity_is_one_time_and_time_limited() {
    let mut once = kit();
    once.redeem(BootstrapChannel::Loopback, InstantMillis::from_millis(1))
        .expect("first");
    assert_eq!(
        once.redeem(BootstrapChannel::Loopback, InstantMillis::from_millis(2))
            .err(),
        Some(KitError::IdentityAlreadyRedeemed)
    );

    let mut expired = kit();
    assert_eq!(
        expired
            .redeem(
                BootstrapChannel::LocalTty,
                InstantMillis::from_millis(10_001)
            )
            .err(),
        Some(KitError::IdentityExpired)
    );
}

#[test]
fn a_kit_stored_inside_the_ordinary_checkpoint_is_refused() {
    let mut kit = kit();
    kit.stored_outside_ordinary_checkpoint = false;

    assert_eq!(
        kit.redeem(BootstrapChannel::LocalTty, InstantMillis::from_millis(1))
            .err(),
        Some(KitError::KitInsideOrdinaryCheckpoint)
    );
}

#[test]
fn no_recovery_write_is_authorized_before_the_transcript_exists() {
    let transcript =
        RecoveryTranscript::begin(CheckpointId::parse("cp-0001").expect("checkpoint id"));

    assert_eq!(
        transcript.authorize_write().err(),
        Some(TranscriptError::TranscriptNotEstablished)
    );
}

#[test]
fn the_transcript_refuses_anything_that_looks_like_a_secret() {
    let mut kit = kit();
    let mut grant = kit
        .redeem(BootstrapChannel::LocalTty, InstantMillis::from_millis(1))
        .expect("redeem");

    assert_eq!(
        grant
            .transcript
            .append(
                RecoveryStep::OwnerAndKeyMappingRestored,
                serde_json::json!({ "password": "hunter2" }),
            )
            .err(),
        Some(TranscriptError::SecretInTranscript)
    );
    assert_eq!(grant.transcript.entries.len(), 1);
}

#[test]
fn tampering_with_any_transcript_entry_breaks_the_integrity_chain() {
    let mut kit = kit();
    let mut grant = kit
        .redeem(BootstrapChannel::LocalTty, InstantMillis::from_millis(1))
        .expect("redeem");
    grant
        .transcript
        .append(
            RecoveryStep::PhysicalRebindIntent,
            serde_json::json!({ "new_root": "/srv/restored/vault" }),
        )
        .expect("append");
    grant.transcript.verify().expect("链自洽");

    grant.transcript.entries[0].entry_hash = DIGEST.to_owned();

    assert_eq!(
        grant.transcript.verify().err(),
        Some(TranscriptError::BrokenChain)
    );
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
    let open = BidirectionalSyncGate {
        divergence_isolated: true,
        manually_reconciled: true,
        new_checkpoint_accepted: true,
        write_and_backup_chain_reaccepted: true,
        old_root_retained: true,
    };
    assert!(open.may_enable());

    for mutate in [
        |gate: &mut BidirectionalSyncGate| gate.divergence_isolated = false,
        |gate: &mut BidirectionalSyncGate| gate.manually_reconciled = false,
        |gate: &mut BidirectionalSyncGate| gate.new_checkpoint_accepted = false,
        |gate: &mut BidirectionalSyncGate| gate.write_and_backup_chain_reaccepted = false,
        |gate: &mut BidirectionalSyncGate| gate.old_root_retained = false,
    ] {
        let mut gate = open;
        mutate(&mut gate);
        assert!(!gate.may_enable());
    }
}
