//! recovery kit 兑换与 transcript 契约：只允许本机 TTY/localhost、一次性、有期限；
//! 任何恢复写入之前先建立带完整性保护且不含秘密的 transcript。

use super::super::support::DIGEST;
use studiolo_backup::checkpoint::manifest::{SecretReference, SecretSource};
use studiolo_backup::recovery::kit::{BootstrapChannel, KitError, RecoveryIdentity, RecoveryKit};
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
