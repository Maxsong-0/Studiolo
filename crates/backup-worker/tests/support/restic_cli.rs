use super::*;
use studiolo_backup::worker::restic::{
    CredentialHolder, JobTag, RepositoryRef, ResticCommand, ResticInvocation, ResticOutcome,
};

#[test]
#[ignore = "仅在108给出已核验官方发行的STUDIOLO_RESTIC_TEST_BINARY时运行"]
fn official_restic_executes_from_original_fd_and_uses_only_explicit_credentials() {
    let path = std::env::var_os("STUDIOLO_RESTIC_TEST_BINARY").unwrap();
    let package =
        ResticPackage::fixture(Path::new(&path), &hash(Path::new(&path)), deadline()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let password = temp.path().join("password");
    fs::write(&password, "fictional-restic-repository-only\n").unwrap();
    fs::set_permissions(&password, fs::Permissions::from_mode(0o600)).unwrap();
    let credentials = ResticCredentials::fixture(&password);
    let version =
        process::run(&package, &["version".into()], None, temp.path(), deadline()).unwrap();
    assert!(version.success);
    assert!(version.stdout.starts_with(b"restic 0.19.1 "));
    let repository =
        RepositoryRef::parse(temp.path().join("repository").to_str().unwrap()).unwrap();
    let argv = vec![
        "--json".into(),
        "--no-cache".into(),
        "--repo".into(),
        repository.as_str().into(),
        "init".into(),
        "--repository-version".into(),
        "2".into(),
    ];
    assert!(
        process::run(&package, &argv, Some(&credentials), temp.path(), deadline())
            .unwrap()
            .success
    );
    assert!(
        !process::run(&package, &argv, Some(&credentials), temp.path(), deadline())
            .unwrap()
            .success
    );
    let source = temp.path().join("sealed");
    fs::create_dir(&source).unwrap();
    let relative = studiolo_core::VaultPath::parse("中文 note;$.txt").unwrap();
    let expected_file = source.join(relative.as_str());
    fs::write(&expected_file, vec![0x5a; 16 * 1024 * 1024]).unwrap();
    let commands = [
        ResticCommand::Backup {
            tag: JobTag::parse("fixed-process-fixture").unwrap(),
            source_root: source.clone(),
        },
        ResticCommand::CheckRepository,
    ];
    for (i, command) in commands.iter().enumerate() {
        let plan = ResticInvocation::plan(
            repository.clone(),
            command,
            CredentialHolder::BackupWorker,
            command.identity(),
        )
        .unwrap();
        let output = process::run(
            &package,
            plan.argv(),
            Some(&credentials),
            temp.path(),
            deadline(),
        )
        .unwrap();
        let parsed = crate::runner::parse_output(&output, i == 0).unwrap();
        assert_eq!(
            parsed,
            ResticOutcome::Exited {
                status_zero: true,
                reported_objects: if i == 0 { 1 } else { 0 }
            }
        );
        if i == 0 {
            let summary: serde_json::Value = output
                .stdout
                .split(|b| *b == b'\n')
                .filter(|line| !line.is_empty())
                .filter_map(|line| serde_json::from_slice::<serde_json::Value>(line).ok())
                .find(|value| value["message_type"] == "summary")
                .unwrap();
            let snapshot = studiolo_backup::worker::restic::SnapshotRef::parse(
                summary["snapshot_id"].as_str().unwrap(),
            )
            .unwrap();
            let argv =
                crate::runner::inspect::plan::file(&repository, &snapshot, &source, &relative)
                    .unwrap();
            let digest = hash(&expected_file);
            let bytes = fs::metadata(&expected_file).unwrap().len();
            for (size, expected, succeeds) in [
                (bytes, digest.as_str(), true),
                (
                    bytes,
                    "0000000000000000000000000000000000000000000000000000000000000000",
                    false,
                ),
                (bytes - 1, digest.as_str(), false),
                (bytes + 1, digest.as_str(), false),
            ] {
                let result = process::verify_digest(
                    &package,
                    &argv,
                    &credentials,
                    temp.path(),
                    deadline(),
                    &|| Ok(()),
                    process::DigestExpectation {
                        bytes: size,
                        sha256: expected,
                    },
                );
                assert_eq!(result.is_ok(), succeeds);
            }
        }
    }
}

#[test]
#[ignore = "仅在108核验官方发行后运行，不使用生产仓库"]
fn official_restic_snapshot_and_complete_tree_match_real_sealed_inventory() {
    use crate::{
        runner::inspect::{listing, plan, snapshot},
        seal_fixture,
    };
    use std::os::unix::fs::MetadataExt;
    let path = std::env::var_os("STUDIOLO_RESTIC_TEST_BINARY").unwrap();
    let package =
        ResticPackage::fixture(Path::new(&path), &hash(Path::new(&path)), deadline()).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let password = temp.path().join("password");
    fs::write(&password, "fictional-fixture-only\n").unwrap();
    let credentials = ResticCredentials::fixture(&password);
    let repository =
        RepositoryRef::parse(temp.path().join("repository").to_str().unwrap()).unwrap();
    let call = |argv: &[String]| {
        let output =
            process::run(&package, argv, Some(&credentials), temp.path(), deadline()).unwrap();
        assert!(output.success);
        output
    };
    call(&[
        "--json".into(),
        "--no-cache".into(),
        "--repo".into(),
        repository.as_str().into(),
        "init".into(),
    ]);
    let fixture = seal_fixture::Fixture::new();
    let sealed = fixture.seal("remote-inventory");
    let root = &sealed.ticket().transfer_source.root;
    let key = seal_fixture::job("remote-inventory", 9);
    let backup = ResticInvocation::plan(
        repository.clone(),
        &ResticCommand::Backup {
            tag: JobTag::for_job(&key).unwrap(),
            source_root: root.clone(),
        },
        CredentialHolder::BackupWorker,
        studiolo_backup::worker::restic::ResticIdentity::Append,
    )
    .unwrap();
    call(backup.argv());
    let output = call(&plan::snapshots(&repository, &key).unwrap());
    let snapshot = snapshot::locate(&output.stdout, &key, root)
        .unwrap()
        .unwrap();
    let listing = call(&plan::listing(&repository, &snapshot).unwrap());
    let stat = fs::metadata(root).unwrap();
    listing::verify(
        &listing.stdout,
        listing::Expected {
            key: &key,
            snapshot: &snapshot,
            root,
            inventory: sealed.inventory(),
            producer: stat.uid(),
            group: stat.gid(),
        },
    )
    .unwrap();
    for file in sealed.inventory().files() {
        process::verify_digest(
            &package,
            &plan::file(&repository, &snapshot, root, file.path()).unwrap(),
            &credentials,
            temp.path(),
            deadline(),
            &|| Ok(()),
            process::DigestExpectation {
                bytes: file.bytes(),
                sha256: file.sha256(),
            },
        )
        .unwrap();
    }
    // 相同作业出现两个候选不能按 latest 猜一个。
    call(backup.argv());
    let duplicate = call(&plan::snapshots(&repository, &key).unwrap());
    assert!(snapshot::locate(&duplicate.stdout, &key, root).is_err());
}
