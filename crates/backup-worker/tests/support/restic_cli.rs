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
    fs::write(source.join("fictional.txt"), b"fictional sealed source").unwrap();
    let commands = [
        ResticCommand::Backup {
            tag: JobTag::parse("fixed-process-fixture").unwrap(),
            source_root: source,
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
    }
}
