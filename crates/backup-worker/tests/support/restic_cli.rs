use super::*;
use studiolo_backup::worker::{
    repository_check::{CheckBucket, CheckOutcome, classify_check_output},
    restic::{
        CredentialHolder, JobTag, RepositoryRef, ResticCommand, ResticIdentity, ResticInvocation,
        ResticOutcome,
    },
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
    let backup = ResticInvocation::plan(
        repository.clone(),
        &ResticCommand::Backup {
            tag: JobTag::parse("fixed-process-fixture").unwrap(),
            source_root: source.clone(),
        },
        CredentialHolder::BackupWorker,
        ResticIdentity::Append,
    )
    .unwrap();
    let output = process::run(
        &package,
        backup.argv(),
        Some(&credentials),
        temp.path(),
        deadline(),
    )
    .unwrap();
    assert_eq!(
        crate::runner::parse_output(&output).unwrap(),
        ResticOutcome::Exited {
            status_zero: true,
            reported_objects: 1
        }
    );
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
        crate::runner::inspect::plan::file(&repository, &snapshot, &source, &relative).unwrap();
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
    // 仓库抽查走 worker 自己的执行入口：不加锁的 `check --read-data-subset`，30 组都归成干净。
    for bucket in CheckBucket::all() {
        let check = ResticInvocation::plan(
            repository.clone(),
            &ResticCommand::CheckSubset { bucket },
            CredentialHolder::BackupWorker,
            ResticIdentity::Append,
        )
        .unwrap();
        let output = process::run_check(
            &package,
            check.argv(),
            &credentials,
            temp.path(),
            deadline(),
            &|| Ok(()),
            &|| Ok(()),
        )
        .unwrap();
        assert_eq!(
            classify_check_output(output.success, &output.stdout),
            CheckOutcome::clean(),
            "第 {} 组",
            bucket.get()
        );
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
    // 最终记录是另一个真实快照，仅 stdin 的固定虚拟文件进入远端。
    use crate::runner::final_record::{plan::Marker, read};
    use studiolo_backup::worker::{final_manifest::FinalManifestDraft, state::RemoteVerification};
    let proof = RemoteVerification {
        objects_complete: true,
        checksum_verified: true,
        paired_checkpoint_reference: true,
        verified_at: studiolo_core::checkpoint::InstantMillis::from_millis(2000),
    };
    let draft = FinalManifestDraft::new(key.clone(), &sealed, snapshot.clone(), proof).unwrap();
    let marker = Marker::for_job(&key).unwrap();
    let read_final = |draft: &FinalManifestDraft| {
        read::read(
            draft,
            &repository,
            &|argv| process::run(&package, argv, Some(&credentials), temp.path(), deadline()),
            &|argv, expected| {
                process::verify_digest(
                    &package,
                    argv,
                    &credentials,
                    temp.path(),
                    deadline(),
                    &|| Ok(()),
                    expected,
                )
            },
        )
    };
    assert!(read_final(&draft).unwrap().is_none());
    let write_final = |marker: &Marker, bytes: &[u8]| {
        let output = process::run_input(
            &package,
            &marker.backup(&repository),
            &credentials,
            temp.path(),
            deadline(),
            &|| Ok(()),
            bytes,
        )
        .unwrap();
        assert!(matches!(
            crate::runner::parse_output(&output).unwrap(),
            ResticOutcome::Exited {
                status_zero: true,
                reported_objects: 1
            }
        ));
    };
    write_final(&marker, &draft.encode().unwrap());
    let final_snapshot = read_final(&draft).unwrap().unwrap();
    assert_ne!(final_snapshot, snapshot);
    // 原路径暂不可读不会影响固定非正文 marker 的读回。
    fs::set_permissions(root, fs::Permissions::from_mode(0o000)).unwrap();
    assert_eq!(read_final(&draft).unwrap().unwrap(), final_snapshot);
    fs::set_permissions(root, fs::Permissions::from_mode(0o550)).unwrap();
    let wrong_key = seal_fixture::job("remote-inventory", 10);
    let wrong =
        FinalManifestDraft::new(wrong_key.clone(), &sealed, snapshot.clone(), proof).unwrap();
    let wrong_marker = Marker::for_job(&wrong_key).unwrap();
    let mut wrong_bytes = wrong.encode().unwrap();
    wrong_bytes[0] = b'[';
    write_final(&wrong_marker, &wrong_bytes);
    assert!(
        read_final(&wrong).is_err(),
        "同长度的错误正文不能冒充确定最终记录"
    );
    write_final(&marker, &draft.encode().unwrap());
    assert!(
        read_final(&draft).is_err(),
        "同作业的多个最终记录不能按 latest 猜选"
    );
    // 相同作业出现两个候选不能按 latest 猜一个。
    call(backup.argv());
    let duplicate = call(&plan::snapshots(&repository, &key).unwrap());
    assert!(snapshot::locate(&duplicate.stdout, &key, root).is_err());
}
