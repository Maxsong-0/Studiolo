//! 恢复点契约：年龄从内容截止时刻算；时钟回退或缺时间证据显示未知；
//! 首次完整 seed 校验前显示「尚未建立灾难恢复点」；
//! 每日不可变恢复点单独计龄；普通 SMB/NFS 目录不得显示具备不可变保护。

use studiolo_backup::worker::recovery_point::{
    BackupTargetKind, ImmutabilityCapability, ImmutabilityUnprovenReason, RecoveryPointDisplay,
    RecoveryPointInputs, RecoveryPointStamps, RecoveryPointStatus, RpoBudget, evaluate_class,
};
use studiolo_core::checkpoint::{AgeUnknownReason, InstantMillis, RecoveryPointAge};

fn stamps(checkpoint_at: i64) -> RecoveryPointStamps {
    RecoveryPointStamps {
        checkpoint_at: InstantMillis::from_millis(checkpoint_at),
        remote_verified_at: InstantMillis::from_millis(checkpoint_at + 60_000),
        finalized_at: InstantMillis::from_millis(checkpoint_at + 120_000),
    }
}

fn inputs() -> RecoveryPointInputs {
    RecoveryPointInputs {
        seed_verified: true,
        scheduled: Some(stamps(1_000_000)),
        daily_immutable: Some(stamps(400_000)),
        target: BackupTargetKind::DedicatedAppendOnlyTarget,
        immutability_proven: true,
        now: Some(InstantMillis::from_millis(1_300_000)),
        budget: RpoBudget::DEFAULT,
    }
}

#[test]
fn the_age_is_measured_from_the_content_cutoff_not_from_the_finalize_time() {
    let status = RecoveryPointStatus::evaluate(inputs());

    assert_eq!(
        status.scheduled,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Known { millis: 300_000 }
        },
        "1_300_000 - checkpoint_at(1_000_000)，不是减去 finalized_at"
    );
}

#[test]
fn the_daily_immutable_recovery_point_is_aged_separately() {
    let status = RecoveryPointStatus::evaluate(inputs());

    assert_eq!(
        status.daily_immutable,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Known { millis: 900_000 }
        }
    );
    assert_ne!(status.scheduled, status.daily_immutable);
}

#[test]
fn a_clock_that_went_backwards_shows_unknown_rather_than_zero() {
    let mut inputs = inputs();
    inputs.now = Some(InstantMillis::from_millis(900_000));

    let status = RecoveryPointStatus::evaluate(inputs);

    assert_eq!(
        status.scheduled,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Unknown {
                reason: AgeUnknownReason::ClockWentBackwards
            }
        }
    );
}

#[test]
fn missing_time_evidence_shows_unknown() {
    let mut without_now = inputs();
    without_now.now = None;
    assert_eq!(
        RecoveryPointStatus::evaluate(without_now).scheduled,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Unknown {
                reason: AgeUnknownReason::MissingTimeEvidence
            }
        }
    );

    let mut without_stamps = inputs();
    without_stamps.scheduled = None;
    assert_eq!(
        RecoveryPointStatus::evaluate(without_stamps).scheduled,
        RecoveryPointDisplay::Established {
            age: RecoveryPointAge::Unknown {
                reason: AgeUnknownReason::MissingTimeEvidence
            }
        }
    );
}

#[test]
fn before_the_first_full_seed_verification_nothing_counts_as_a_recovery_point() {
    let mut inputs = inputs();
    inputs.seed_verified = false;

    let status = RecoveryPointStatus::evaluate(inputs);

    assert_eq!(status.scheduled, RecoveryPointDisplay::NotEstablished);
    assert_eq!(status.daily_immutable, RecoveryPointDisplay::NotEstablished);
    assert!(!status.scheduled.is_established());
    assert_eq!(
        evaluate_class(false, Some(stamps(0)), Some(InstantMillis::from_millis(1))),
        RecoveryPointDisplay::NotEstablished
    );
}

#[test]
fn a_plain_smb_or_nfs_directory_never_shows_immutable_protection() {
    let mut inputs = inputs();
    inputs.target = BackupTargetKind::PlainSmbOrNfsDirectory;
    inputs.immutability_proven = true;

    let status = RecoveryPointStatus::evaluate(inputs);

    assert_eq!(
        status.immutability,
        ImmutabilityCapability::NotProven {
            reason: ImmutabilityUnprovenReason::PlainDirectoryTargetHasNoImmutability
        },
        "声明本身造不出不可变保护"
    );
    assert!(!status.shows_immutable_protection());
}

#[test]
fn a_local_temporary_fixture_is_never_presented_as_a_disaster_recovery_target() {
    let mut inputs = inputs();
    inputs.target = BackupTargetKind::LocalTemporaryRepository;

    let status = RecoveryPointStatus::evaluate(inputs);

    assert_eq!(
        status.immutability,
        ImmutabilityCapability::NotProven {
            reason: ImmutabilityUnprovenReason::LocalTemporaryFixture
        }
    );
}

#[test]
fn a_dedicated_target_that_is_not_connected_yet_stays_unproven() {
    let mut inputs = inputs();
    inputs.immutability_proven = false;

    let status = RecoveryPointStatus::evaluate(inputs);

    assert_eq!(
        status.immutability,
        ImmutabilityCapability::NotProven {
            reason: ImmutabilityUnprovenReason::DedicatedTargetNotConnected
        }
    );
}

#[test]
fn the_default_ten_minute_schedule_plus_five_minute_budget_meets_the_fifteen_minute_rpo() {
    assert!(RpoBudget::DEFAULT.is_satisfied());
    assert_eq!(RpoBudget::DEFAULT.schedule_interval_millis, 600_000);
    assert_eq!(RpoBudget::DEFAULT.completion_budget_millis, 300_000);
    assert_eq!(RpoBudget::DEFAULT.rpo_millis, 900_000);
    assert!(RecoveryPointStatus::evaluate(inputs()).rpo_satisfied);
}

#[test]
fn a_budget_that_overruns_the_rpo_or_overflows_is_not_satisfied() {
    let mut slow = RpoBudget::DEFAULT;
    slow.completion_budget_millis = 300_001;
    assert!(!slow.is_satisfied());

    let overflowing = RpoBudget {
        schedule_interval_millis: u64::MAX,
        completion_budget_millis: 1,
        rpo_millis: u64::MAX,
    };
    assert!(!overflowing.is_satisfied(), "相加溢出一律判为不满足");
}
