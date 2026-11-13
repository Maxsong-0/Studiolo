use crate::support::{AuthorityViolation, source_violations, workspace_root};
use std::{fs, path::Path};

#[test]
fn policy_checkpoint_only_opens_fixed_database_and_only_storage_inspects_target_directory() {
    let root = workspace_root();
    let storage = "crates/policy/src/storage.rs";
    let source = fs::read_to_string(root.join(storage)).unwrap();
    assert!(source_violations("studiolo-policy", Path::new(storage), &source).is_empty());
    for extra in [
        "fs::read_dir(other)",
        "fs::remove_file(path)",
        "fs::write(path, bytes)",
    ] {
        assert!(
            source_violations(
                "studiolo-policy",
                Path::new(storage),
                &format!("{source}\n{extra}")
            )
            .contains(&AuthorityViolation::GenericWrite)
        );
    }
    let connection = "crates/policy/src/database.rs";
    assert!(
        source_violations(
            "studiolo-policy",
            Path::new(connection),
            &fs::read_to_string(root.join(connection)).unwrap()
        )
        .is_empty()
    );
    for relative in [
        "crates/policy/src/checkpoint.rs",
        "crates/policy/src/checkpoint/snapshot.rs",
    ] {
        let source = fs::read_to_string(root.join(relative)).unwrap();
        assert!(source_violations("studiolo-policy", Path::new(relative), &source).is_empty());
        for forbidden in [
            "VACUUM INTO",
            "wal_checkpoint(PASSIVE)",
            "BEGIN IMMEDIATE",
            "Connection::open",
            "OpenOptions::new",
            "fs::",
        ] {
            assert!(!source.contains(forbidden), "{relative}: {forbidden}");
        }
    }
}
