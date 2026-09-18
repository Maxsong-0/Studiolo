//! restic 契约：固定包与封闭参数，追加凭据只属于 BackupWorker，
//! 恢复与保留清理使用独立身份，任何 shell 形态的参数都进不来。

use std::path::PathBuf;
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
