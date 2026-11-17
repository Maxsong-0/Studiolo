//! restic 契约：固定包与封闭参数，追加凭据只属于 BackupWorker，
//! 恢复与保留清理使用独立身份，任何 shell 形态的参数都进不来。

use std::path::PathBuf;
use studiolo_backup::worker::repository_check::CheckBucket;
use studiolo_backup::worker::restic::{
    CredentialHolder, JobTag, RESTIC_PACKAGE_NAME, RESTIC_PACKAGE_VERSION,
    RESTIC_SECRET_ENVIRONMENT, RepositoryRef, ResticCommand, ResticError, ResticIdentity,
    ResticInvocation, ResticOutcome, ResticRunner, SnapshotRef, UnconfiguredResticRunner,
    UnverifiedReason,
};

fn repository() -> RepositoryRef {
    RepositoryRef::parse("/srv/backup/repo").expect("repository")
}

fn backup_command() -> ResticCommand {
    ResticCommand::Backup {
        tag: JobTag::parse("job-1").expect("tag"),
        source_root: PathBuf::from("/srv/backup-staging/cp-1"),
    }
}

#[test]
fn a_backup_invocation_pins_the_package_version_and_a_closed_argument_vector() {
    let invocation = ResticInvocation::plan(
        repository(),
        &backup_command(),
        CredentialHolder::BackupWorker,
        ResticIdentity::Append,
    )
    .expect("plan");

    assert_eq!(invocation.package(), RESTIC_PACKAGE_NAME);
    assert_eq!(invocation.version(), RESTIC_PACKAGE_VERSION);
    assert_eq!(
        invocation.argv(),
        [
            "--json",
            "--no-cache",
            "--repo",
            "/srv/backup/repo",
            "backup",
            "--tag",
            "job-1",
            "--one-file-system",
            "/srv/backup-staging/cp-1",
        ]
    );
    assert_eq!(invocation.secret_environment(), RESTIC_SECRET_ENVIRONMENT);
    assert!(
        invocation
            .argv()
            .iter()
            .all(|argument| !argument.contains(';') && !argument.contains(' ')),
        "参数向量里不可能出现 shell 片段"
    );
}

#[test]
fn checking_reads_one_of_thirty_buckets_without_a_lock_and_claims_no_snapshot_filter() {
    let plan = |bucket| {
        ResticInvocation::plan(
            repository(),
            &ResticCommand::CheckSubset { bucket },
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )
        .unwrap()
    };
    for (bucket, subset) in [(1, "1/30"), (17, "17/30"), (30, "30/30")] {
        let invocation = plan(CheckBucket::new(bucket).unwrap());
        assert_eq!(
            invocation.argv(),
            [
                "--json",
                "--no-cache",
                "--repo",
                "/srv/backup/repo",
                "check",
                "--no-lock",
                "--read-data-subset",
                subset,
            ]
        );
        assert_eq!(invocation.version(), "0.19.1");
        assert_eq!(invocation.identity(), ResticIdentity::Append);
    }
    // 组号只能是 1～30：0、31 和越界的值连组号都造不出来，持久化的值也绕不过。
    for value in [0, 31, 255] {
        assert!(CheckBucket::new(value).is_none());
        assert!(serde_json::from_str::<CheckBucket>(&value.to_string()).is_err());
    }
    assert_eq!(serde_json::from_str::<CheckBucket>("7").unwrap().get(), 7);
    // 追加身份以外的持有者和身份照样不能组装抽查。
    assert!(
        ResticInvocation::plan(
            repository(),
            &ResticCommand::CheckSubset {
                bucket: CheckBucket::FIRST
            },
            CredentialHolder::App,
            ResticIdentity::Append,
        )
        .is_err()
    );
    assert!(
        ResticInvocation::plan(
            repository(),
            &ResticCommand::CheckSubset {
                bucket: CheckBucket::FIRST
            },
            CredentialHolder::BackupWorker,
            ResticIdentity::Restore,
        )
        .is_err()
    );
}

#[test]
fn persisted_arguments_cannot_bypass_parsing_and_remote_tags_bind_checkpoint_identity() {
    assert!(serde_json::from_str::<RepositoryRef>("\"relative\"").is_err());
    assert!(serde_json::from_str::<JobTag>("\"--exec=anything\"").is_err());
    assert!(serde_json::from_str::<SnapshotRef>("\"latest\"").is_err());
    let key = |checkpoint| studiolo_backup::worker::state::JobKey {
        sequence: std::num::NonZeroU64::new(u64::MAX).unwrap(),
        checkpoint_id: studiolo_core::checkpoint::CheckpointId::parse(checkpoint).unwrap(),
    };
    let a = JobTag::for_job(&key("original")).unwrap();
    let b = JobTag::for_job(&key("new restoration")).unwrap();
    assert_ne!(a, b);
    assert_eq!(a, JobTag::for_job(&key("original")).unwrap());
    assert!(a.as_str().len() <= 64);
}

#[test]
fn only_the_backup_worker_may_hold_the_append_credential() {
    for holder in [
        CredentialHolder::App,
        CredentialHolder::Broker,
        CredentialHolder::Auth,
        CredentialHolder::Audit,
        CredentialHolder::TaskEngine,
    ] {
        assert_eq!(
            ResticInvocation::plan(
                repository(),
                &backup_command(),
                holder,
                ResticIdentity::Append,
            ),
            Err(ResticError::CredentialHolderForbidden { holder })
        );
    }
}

#[test]
fn restore_and_retention_cleanup_use_identities_separate_from_the_append_one() {
    let restore = ResticCommand::Restore {
        snapshot: SnapshotRef::parse("0a1b2c3d").expect("snapshot"),
        target_root: PathBuf::from("/srv/restored/vault"),
    };
    assert_eq!(restore.identity(), ResticIdentity::Restore);
    assert_eq!(
        ResticCommand::Forget { keep_last: 7 }.identity(),
        ResticIdentity::RetentionCleanup
    );

    assert_eq!(
        ResticInvocation::plan(
            repository(),
            &restore,
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        ),
        Err(ResticError::IdentityMismatch {
            expected: ResticIdentity::Restore,
            found: ResticIdentity::Append,
        })
    );

    let planned = ResticInvocation::plan(
        repository(),
        &restore,
        CredentialHolder::BackupWorker,
        ResticIdentity::Restore,
    )
    .expect("plan");
    assert_eq!(planned.identity(), ResticIdentity::Restore);
}

#[test]
fn a_tag_or_snapshot_carrying_shell_metacharacters_never_parses() {
    for bad in ["job 1; rm -rf /", "job`id`", "job$(id)", "job|cat", ""] {
        assert_eq!(
            JobTag::parse(bad),
            Err(ResticError::ForbiddenArgument),
            "{bad}"
        );
    }
    for bad in ["0a1b", "zzzzzzzz", "0a1b2c3d;rm"] {
        assert_eq!(
            SnapshotRef::parse(bad),
            Err(ResticError::ForbiddenArgument),
            "{bad}"
        );
    }
}

#[test]
fn a_repository_locator_must_take_one_of_the_fixed_shapes() {
    RepositoryRef::parse("/srv/backup/repo").expect("local path");
    RepositoryRef::parse("rest:https://backup.example/repo").expect("rest");
    RepositoryRef::parse("sftp:backup@host:/srv/repo").expect("sftp");
    for bad in [
        "repo",
        "file://srv/repo",
        "/srv/repo; curl http://evil",
        "$(cat /etc/passwd)",
    ] {
        assert_eq!(
            RepositoryRef::parse(bad),
            Err(ResticError::ForbiddenArgument),
            "{bad}"
        );
    }
}

#[test]
fn a_relative_or_traversing_source_root_is_refused() {
    for bad in ["relative/path", "/srv/../etc", "/srv/staging/.."] {
        let command = ResticCommand::Backup {
            tag: JobTag::parse("job-1").expect("tag"),
            source_root: PathBuf::from(bad),
        };
        assert_eq!(
            ResticInvocation::plan(
                repository(),
                &command,
                CredentialHolder::BackupWorker,
                ResticIdentity::Append,
            ),
            Err(ResticError::ForbiddenArgument),
            "{bad}"
        );
    }
}

#[test]
fn without_a_real_repository_the_runner_reports_unverified_rather_than_success() {
    let invocation = ResticInvocation::plan(
        repository(),
        &backup_command(),
        CredentialHolder::BackupWorker,
        ResticIdentity::Append,
    )
    .expect("plan");

    let outcome = UnconfiguredResticRunner.run(&invocation).expect("run");

    assert_eq!(
        outcome,
        ResticOutcome::Unverified {
            reason: UnverifiedReason::NoRealRepository
        },
        "拿不到真实仓库就明确返回未验证，不降级伪装成功"
    );
}
