//! 状态机契约：九种状态互不混淆；上传退出 0 只是中间证据；
//! 未知结果沿原作业键核对；审计待确认保留真实状态且不推进成功指针。

use std::num::NonZeroU64;
use studiolo_backup::worker::state::{
    FailureReason, JobEvent, JobKey, JobRecord, JobState, ReconciledOutcome, RemoteVerification,
    StateError, UnknownStage, UploadEvidence,
};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis};

fn key(sequence: u64) -> JobKey {
    JobKey {
        sequence: NonZeroU64::new(sequence).expect("sequence"),
        checkpoint_id: CheckpointId::parse("cp-0001").expect("checkpoint id"),
    }
}

fn good_verification() -> RemoteVerification {
    RemoteVerification {
        objects_complete: true,
        checksum_verified: true,
        paired_checkpoint_reference: true,
        verified_at: InstantMillis::from_millis(2_000),
    }
}

fn through_upload() -> JobRecord {
    let mut job = JobRecord::queued(key(1));
    job.advance(JobEvent::StartSealing {
        checkpoint_at: InstantMillis::from_millis(1_000),
    })
    .expect("sealing");
    job.advance(JobEvent::SealCompleted).expect("uploading");
    job
}

#[test]
fn the_happy_path_walks_every_intermediate_state_before_reporting_success() {
    let mut job = through_upload();
    assert_eq!(job.state, JobState::Uploading);

    job.advance(JobEvent::UploadExited {
        evidence: UploadEvidence {
            exit_status_zero: true,
            reported_objects: 3,
        },
    })
    .expect("verifying");
    assert_eq!(
        job.state,
        JobState::Verifying,
        "上传退出 0 只能进入校验，不能直接成功"
    );

    job.advance(JobEvent::RemoteVerified {
        verification: good_verification(),
    })
    .expect("committing");
    assert_eq!(job.state, JobState::CommittingFinalManifest);
    assert!(
        !job.state.advances_success_pointer(),
        "最终清单提交前没有可恢复点"
    );

    job.advance(JobEvent::FinalManifestCommitted {
        finalized_at: InstantMillis::from_millis(3_000),
        outcome_audit_succeeded: true,
    })
    .expect("succeeded");
    assert_eq!(job.state, JobState::Succeeded);
    assert!(job.state.advances_success_pointer());
    assert_eq!(job.checkpoint_at, Some(InstantMillis::from_millis(1_000)));
    assert_eq!(
        job.remote_verified_at,
        Some(InstantMillis::from_millis(2_000))
    );
    assert_eq!(job.finalized_at, Some(InstantMillis::from_millis(3_000)));
}

#[test]
fn an_incomplete_remote_object_cannot_produce_a_recovery_point() {
    for broken in 0..3 {
        let mut job = through_upload();
        job.advance(JobEvent::UploadExited {
            evidence: UploadEvidence {
                exit_status_zero: true,
                reported_objects: 3,
            },
        })
        .expect("verifying");
        let mut verification = good_verification();
        match broken {
            0 => verification.objects_complete = false,
            1 => verification.checksum_verified = false,
            _ => verification.paired_checkpoint_reference = false,
        }
        job.advance(JobEvent::RemoteVerified { verification })
            .expect("transition");
        assert_eq!(
            job.state,
            JobState::Failed {
                reason: FailureReason::RemoteVerificationFailed
            },
            "broken={broken}"
        );
        assert!(job.remote_verified_at.is_none());
    }
}

#[test]
fn a_nonzero_upload_exit_is_a_failure_rather_than_an_unknown_result() {
    let mut job = through_upload();
    job.advance(JobEvent::UploadExited {
        evidence: UploadEvidence {
            exit_status_zero: false,
            reported_objects: 0,
        },
    })
    .expect("transition");
    assert_eq!(
        job.state,
        JobState::Failed {
            reason: FailureReason::UploadRejected
        }
    );
}

#[test]
fn an_interrupted_upload_becomes_result_unknown_and_stays_out_of_both_terminals() {
    let mut job = through_upload();
    job.advance(JobEvent::ResultUnknown {
        stage: UnknownStage::Uploading,
    })
    .expect("unknown");

    assert_eq!(
        job.state,
        JobState::ResultUnknown {
            stage: UnknownStage::Uploading
        }
    );
    assert!(!job.state.is_terminal());
    assert!(!job.state.advances_success_pointer());
}

#[test]
fn an_unknown_result_is_reconciled_only_under_its_original_job_key() {
    let mut job = through_upload();
    job.advance(JobEvent::ResultUnknown {
        stage: UnknownStage::FinalManifestAck,
    })
    .expect("unknown");

    assert_eq!(
        job.reconcile_unknown(
            &key(2),
            ReconciledOutcome::RemoteVerified {
                verification: good_verification()
            },
        ),
        Err(StateError::JobKeyMismatch),
        "不能用别的键去核对"
    );

    job.reconcile_unknown(
        &key(1),
        ReconciledOutcome::RemoteVerified {
            verification: good_verification(),
        },
    )
    .expect("reconcile");
    assert_eq!(job.state, JobState::CommittingFinalManifest);
}

#[test]
fn a_lost_ack_whose_object_is_absent_remotely_settles_as_a_failure() {
    let mut job = through_upload();
    job.advance(JobEvent::ResultUnknown {
        stage: UnknownStage::FinalManifestAck,
    })
    .expect("unknown");

    job.reconcile_unknown(&key(1), ReconciledOutcome::RemoteAbsent)
        .expect("reconcile");

    assert_eq!(
        job.state,
        JobState::Failed {
            reason: FailureReason::UploadRejected
        }
    );
}

#[test]
fn a_failed_outcome_audit_keeps_the_real_remote_state_without_moving_the_success_pointer() {
    let mut job = through_upload();
    job.advance(JobEvent::UploadExited {
        evidence: UploadEvidence {
            exit_status_zero: true,
            reported_objects: 3,
        },
    })
    .expect("verifying");
    job.advance(JobEvent::RemoteVerified {
        verification: good_verification(),
    })
    .expect("committing");

    job.advance(JobEvent::FinalManifestCommitted {
        finalized_at: InstantMillis::from_millis(3_000),
        outcome_audit_succeeded: false,
    })
    .expect("audit pending");

    let JobState::AuditPending { verification } = job.state else {
        panic!("远端已确认必须保留为 AuditPending：{:?}", job.state);
    };
    assert!(verification.is_recoverable(), "不能声称上传没发生");
    assert!(!job.state.advances_success_pointer());
    assert!(!job.state.is_terminal());

    job.advance(JobEvent::OutcomeBackfilled).expect("backfill");
    assert_eq!(job.state, JobState::Succeeded);
}

#[test]
fn terminal_states_refuse_every_further_event() {
    let mut job = through_upload();
    job.advance(JobEvent::Failed {
        reason: FailureReason::OutOfSpace,
    })
    .expect("failed");

    assert!(matches!(
        job.advance(JobEvent::SealCompleted),
        Err(StateError::IllegalTransition { .. })
    ));
    assert!(matches!(
        job.advance(JobEvent::ResultUnknown {
            stage: UnknownStage::Sealing
        }),
        Err(StateError::IllegalTransition { .. })
    ));
    assert_eq!(
        job.state,
        JobState::Failed {
            reason: FailureReason::OutOfSpace
        }
    );
}

#[test]
fn stages_cannot_be_skipped() {
    let mut job = JobRecord::queued(key(1));
    assert!(matches!(
        job.advance(JobEvent::UploadExited {
            evidence: UploadEvidence {
                exit_status_zero: true,
                reported_objects: 1
            }
        }),
        Err(StateError::IllegalTransition { .. })
    ));
    assert!(matches!(
        job.advance(JobEvent::FinalManifestCommitted {
            finalized_at: InstantMillis::from_millis(1),
            outcome_audit_succeeded: true,
        }),
        Err(StateError::IllegalTransition { .. })
    ));
    assert_eq!(job.state, JobState::Queued);
}

#[test]
fn a_source_change_before_sealing_is_a_named_failure_rather_than_a_silent_retry() {
    let mut job = JobRecord::queued(key(1));
    job.advance(JobEvent::StartSealing {
        checkpoint_at: InstantMillis::from_millis(1_000),
    })
    .expect("sealing");

    job.advance(JobEvent::Failed {
        reason: FailureReason::SourceChangedBeforeSeal,
    })
    .expect("failed");

    assert_eq!(
        job.state,
        JobState::Failed {
            reason: FailureReason::SourceChangedBeforeSeal
        }
    );
}

#[test]
fn the_job_key_is_a_monotonic_sequence_not_a_content_hash() {
    let first = key(1);
    let second = key(2);
    assert!(second.sequence > first.sequence);
    let encoded = serde_json::to_value(&first).expect("encode");
    assert_eq!(encoded["sequence"], 1);
    assert_eq!(encoded["checkpoint_id"], "cp-0001");
}
