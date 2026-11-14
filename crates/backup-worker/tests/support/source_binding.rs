//! 封存由真实 staging 产生，owner 水位是明确的 fixture；跨 UID 读取另行演练。
use super::*;
use crate::seal_fixture::*;
use std::{fs, os::unix::fs::PermissionsExt};

#[test]
fn job_key_reference_repository_tag_and_source_are_one_binding() {
    let f = Fixture::new();
    let sealed = f.seal("cp-binding");
    let reference = sealed.reference().clone();
    let key = job("cp-binding", 7);
    let binding = Binding::new(repo(), key.clone(), &reference, sealed).unwrap();
    assert_eq!(
        binding.accepts(&plan(
            repo(),
            &key,
            &binding.sealed.ticket().transfer_source.root
        )),
        Ok(true)
    );
    let check = ResticInvocation::plan(
        repo(),
        &ResticCommand::CheckRepository,
        CredentialHolder::BackupWorker,
        ResticIdentity::Append,
    )
    .unwrap();
    assert_eq!(binding.accepts(&check), Ok(false));
    for invocation in [
        plan(
            repo(),
            &job("cp-binding", 8),
            &binding.sealed.ticket().transfer_source.root,
        ),
        plan(
            repo(),
            &job("other", 7),
            &binding.sealed.ticket().transfer_source.root,
        ),
        plan(repo(), &key, &f.parent),
        plan(
            RepositoryRef::parse("rest:https://other.example.test/repo/").unwrap(),
            &key,
            &binding.sealed.ticket().transfer_source.root,
        ),
    ] {
        assert_eq!(
            binding.accepts(&invocation),
            Err(ProcessError::Configuration)
        );
    }
    assert!(
        Binding::new(
            repo(),
            job("other", 7),
            &reference,
            f.reopen(&reference).unwrap()
        )
        .is_err()
    );
    let mut wrong = reference.clone();
    wrong.seal_sha256 = "00".repeat(32);
    assert!(Binding::new(repo(), key, &wrong, f.reopen(&reference).unwrap()).is_err());
}
#[test]
fn reopens_exact_seal_and_refuses_replacement_or_modified_payload() {
    let f = Fixture::new();
    let sealed = f.seal("first");
    let reference = sealed.reference().clone();
    let binding = Binding::new(repo(), job("first", 1), &reference, sealed).unwrap();
    assert_eq!(
        binding.check_reopened(&f.reopen(&reference).unwrap()),
        Ok(())
    );
    assert_eq!(
        binding.check_reopened(&f.seal("other")),
        Err(ProcessError::Source)
    );
    let note = binding
        .sealed
        .ticket()
        .transfer_source
        .root
        .join("data/canonical/note");
    fs::set_permissions(&note, fs::Permissions::from_mode(0o640)).unwrap();
    fs::write(&note, b"modified payload").unwrap();
    fs::set_permissions(&note, fs::Permissions::from_mode(0o440)).unwrap();
    assert!(f.reopen(&reference).is_err());
}
