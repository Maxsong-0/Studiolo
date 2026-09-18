//! 只读投影契约：未知不显示绿色，未验证不写成已通过，
//! 远端已确认但审计待补记时明确说明成功指针没有前移。

use super::super::evidence::BackupSnapshot;
use super::{BackupSection, Severity};
use studiolo_backup::worker::recovery_point::{
    BackupTargetKind, RecoveryPointInputs, RecoveryPointStamps, RecoveryPointStatus, RpoBudget,
};
use studiolo_backup::worker::state::{FailureReason, JobState, RemoteVerification, UnknownStage};
use studiolo_core::checkpoint::InstantMillis;

fn status(seed_verified: bool, now: Option<i64>) -> RecoveryPointStatus {
    RecoveryPointStatus::evaluate(RecoveryPointInputs {
        seed_verified,
        scheduled: Some(RecoveryPointStamps {
            checkpoint_at: InstantMillis::from_millis(1_000_000),
            remote_verified_at: InstantMillis::from_millis(1_060_000),
            finalized_at: InstantMillis::from_millis(1_120_000),
        }),
        daily_immutable: None,
        target: BackupTargetKind::PlainSmbOrNfsDirectory,
        immutability_proven: true,
        now: now.map(InstantMillis::from_millis),
        budget: RpoBudget::DEFAULT,
    })
}

fn snapshot(seed_verified: bool, now: Option<i64>, last_job: Option<JobState>) -> BackupSnapshot {
    BackupSnapshot {
        status: status(seed_verified, now),
        approved: true,
        last_job,
    }
}

#[test]
fn a_verified_recent_recovery_point_reads_as_ok_with_its_age_from_the_content_cutoff() {
    let section =
        BackupSection::from_snapshot(&snapshot(true, Some(1_120_000), Some(JobState::Succeeded)));

    assert_eq!(section.severity, Severity::Ok);
    assert_eq!(
        section.recovery_point,
        "恢复点年龄 120 秒（自内容截止时刻起算）"
    );
    assert_eq!(section.last_job, "最近一次作业成功");
    assert_eq!(section.rpo, "调度与完成预算满足声明的 RPO");
}

#[test]
fn before_the_first_seed_verification_the_page_says_no_disaster_recovery_point_yet() {
    let section =
        BackupSection::from_snapshot(&snapshot(false, Some(1_120_000), Some(JobState::Succeeded)));

    assert_eq!(section.recovery_point, "尚未建立灾难恢复点");
    assert_eq!(section.daily_immutable, "尚未建立灾难恢复点");
    assert_ne!(section.severity, Severity::Ok, "未建立时不得显示绿色");
}

#[test]
fn a_clock_that_went_backwards_is_reported_as_unknown_rather_than_zero() {
    let section =
        BackupSection::from_snapshot(&snapshot(true, Some(900_000), Some(JobState::Succeeded)));

    assert_eq!(section.recovery_point, "恢复点年龄未知：本地时钟发生回退");
    assert_eq!(section.severity, Severity::Unknown);
}

#[test]
fn missing_time_evidence_is_reported_as_unknown() {
    let section = BackupSection::from_snapshot(&snapshot(true, None, Some(JobState::Succeeded)));

    assert_eq!(section.recovery_point, "恢复点年龄未知：缺少时间证据");
    assert_eq!(section.severity, Severity::Unknown);
}

#[test]
fn a_plain_directory_target_never_reads_as_having_immutable_protection() {
    let section =
        BackupSection::from_snapshot(&snapshot(true, Some(1_120_000), Some(JobState::Succeeded)));

    assert_eq!(
        section.immutable_protection,
        "未证明具备不可变保护：普通 SMB/NFS 目录不提供该能力"
    );
}

#[test]
fn an_audit_pending_job_says_the_success_pointer_has_not_moved() {
    let section = BackupSection::from_snapshot(&snapshot(
        true,
        Some(1_120_000),
        Some(JobState::AuditPending {
            verification: RemoteVerification {
                objects_complete: true,
                checksum_verified: true,
                paired_checkpoint_reference: true,
                verified_at: InstantMillis::from_millis(1_060_000),
            },
        }),
    ));

    assert!(section.last_job.contains("成功指针未前移"));
    assert!(section.last_job.contains("远端已确认"));
    assert_eq!(section.severity, Severity::Unknown);
}

#[test]
fn an_unknown_result_is_reported_together_with_the_reconciliation_rule() {
    let section = BackupSection::from_snapshot(&snapshot(
        true,
        Some(1_120_000),
        Some(JobState::ResultUnknown {
            stage: UnknownStage::Uploading,
        }),
    ));

    assert!(section.last_job.contains("沿原作业键核对"));
    assert_eq!(section.severity, Severity::Unknown);
}

#[test]
fn a_failed_job_or_an_unmet_rpo_reads_as_attention() {
    let failed = BackupSection::from_snapshot(&snapshot(
        true,
        Some(1_120_000),
        Some(JobState::Failed {
            reason: FailureReason::TargetUnverified,
        }),
    ));
    assert_eq!(failed.severity, Severity::Attention);

    let mut unmet = snapshot(true, Some(1_120_000), Some(JobState::Succeeded));
    unmet.status.rpo_satisfied = false;
    let section = BackupSection::from_snapshot(&unmet);
    assert_eq!(section.severity, Severity::Attention);
    assert_eq!(section.rpo, "调度与完成预算不满足声明的 RPO");
}

#[test]
fn an_unapproved_or_absent_backup_shows_unknown_everywhere() {
    let mut unapproved = snapshot(true, Some(1_120_000), Some(JobState::Succeeded));
    unapproved.approved = false;
    let section = BackupSection::from_snapshot(&unapproved);

    assert_eq!(section.severity, Severity::Unknown);
    assert_eq!(section.recovery_point, "尚未建立灾难恢复点");
    assert_eq!(section.last_job, "备份未获主人批准，未启用");

    let unavailable = BackupSection::unavailable();
    assert_eq!(unavailable.severity, Severity::Unknown);
    assert_eq!(unavailable.immutable_protection, "未证明具备不可变保护");
}

#[test]
fn the_section_serializes_without_leaking_paths_or_credentials() {
    let section =
        BackupSection::from_snapshot(&snapshot(true, Some(1_120_000), Some(JobState::Succeeded)));
    let encoded = serde_json::to_string(&section).expect("encode");

    assert!(!encoded.contains("RESTIC"));
    // 主机路径长成 `/srv/...` 这样：斜杠后紧跟小写字母。
    // 「SMB/NFS」这种协议名不算，因为斜杠后是大写。
    let bytes = encoded.as_bytes();
    assert!(
        !bytes
            .windows(2)
            .any(|pair| pair[0] == b'/' && pair[1].is_ascii_lowercase()),
        "文案里不出现主机路径：{encoded}"
    );
}
