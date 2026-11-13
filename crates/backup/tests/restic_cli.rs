//! 只在测试机使用经过发行摘要核验的真实 restic；仓库/口令/内容均为一次性虚构数据。
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};
use studiolo_backup::worker::{
    restic::{
        CredentialHolder, JobTag, RepositoryRef, ResticCommand, ResticIdentity, ResticInvocation,
        SnapshotRef,
    },
    state::JobKey,
};
use studiolo_core::checkpoint::CheckpointId;

fn invoke(binary: &Path, arguments: &[String], temporary: &Path) -> Output {
    Command::new(binary)
        .args(arguments)
        .env_clear()
        .env("RESTIC_PASSWORD", "fictional-test-repository-only")
        .env("TMPDIR", temporary)
        .output()
        .expect("执行真实 restic")
}
fn plan(repository: &RepositoryRef, command: &ResticCommand) -> ResticInvocation {
    ResticInvocation::plan(
        repository.clone(),
        command,
        CredentialHolder::BackupWorker,
        command.identity(),
    )
    .unwrap()
}
fn require_success(output: Output) -> Vec<u8> {
    assert!(
        output.status.success(),
        "真实 CLI 失败：{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
#[test]
#[ignore = "仅在 108 用核验官方发行摘要后的 STUDIOLO_RESTIC_TEST_BINARY 运行"]
fn pinned_cli_creates_checks_and_restores_a_real_repository_with_planned_arguments() {
    let binary = std::env::var_os("STUDIOLO_RESTIC_TEST_BINARY").expect("必须提供已核验的固定 CLI");
    let binary = Path::new(&binary);
    let temporary = tempfile::tempdir().unwrap();
    let root = temporary.path();
    let version = require_success(invoke(binary, &["version".into()], root));
    assert!(
        String::from_utf8(version)
            .unwrap()
            .starts_with("restic 0.19.1 ")
    );
    let repo = root.join("repository");
    let repository = RepositoryRef::parse(repo.to_str().unwrap()).unwrap();
    require_success(invoke(
        binary,
        &[
            "--json".into(),
            "--no-cache".into(),
            "--repo".into(),
            repository.as_str().into(),
            "init".into(),
            "--repository-version".into(),
            "2".into(),
        ],
        root,
    ));
    let source = root.join("sealed");
    fs::create_dir(&source).unwrap();
    let content = b"fictional sealed object; no production data";
    fs::write(source.join("evidence.txt"), content).unwrap();
    let tag = JobTag::for_job(&JobKey {
        sequence: std::num::NonZeroU64::new(1).unwrap(),
        checkpoint_id: CheckpointId::parse("cli-checkpoint").unwrap(),
    })
    .unwrap();
    let backup = plan(
        &repository,
        &ResticCommand::Backup {
            tag: tag.clone(),
            source_root: source.clone(),
        },
    );
    assert_eq!(backup.identity(), ResticIdentity::Append);
    let output = require_success(invoke(binary, backup.argv(), root));
    let records: Vec<serde_json::Value> = String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let summaries: Vec<_> = records
        .iter()
        .filter(|record| record["message_type"] == "summary")
        .collect();
    assert_eq!(summaries.len(), 1);
    let snapshot = summaries[0]["snapshot_id"].as_str().unwrap();
    assert_eq!(snapshot.len(), 64);
    let snapshot = SnapshotRef::parse(snapshot).unwrap();
    let snapshots = require_success(invoke(
        binary,
        &[
            "--json".into(),
            "--no-cache".into(),
            "--repo".into(),
            repository.as_str().into(),
            "snapshots".into(),
            "--tag".into(),
            tag.as_str().into(),
        ],
        root,
    ));
    let snapshots: serde_json::Value = serde_json::from_slice(&snapshots).unwrap();
    assert_eq!(snapshots.as_array().unwrap().len(), 1);
    assert_eq!(snapshots[0]["id"], snapshot.as_str());
    let check = plan(&repository, &ResticCommand::CheckRepository);
    require_success(invoke(binary, check.argv(), root));
    let restored = root.join("restored");
    let restore = plan(
        &repository,
        &ResticCommand::Restore {
            snapshot,
            target_root: restored.clone(),
        },
    );
    require_success(invoke(binary, restore.argv(), root));
    let restored_file = restored
        .join(source.strip_prefix("/").unwrap())
        .join("evidence.txt");
    assert_eq!(fs::read(restored_file).unwrap(), content);

    // 在本测试生成的私有仓库模拟盘损坏，生产仓库及其权限不参与此测试。
    let pack = fs::read_dir(repo.join("data"))
        .unwrap()
        .flat_map(|dir| fs::read_dir(dir.unwrap().path()).unwrap())
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = fs::read(&pack).unwrap();
    bytes[16] ^= 1;
    fs::set_permissions(&pack, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(pack, bytes).unwrap();
    assert!(!invoke(binary, check.argv(), root).status.success());
}
