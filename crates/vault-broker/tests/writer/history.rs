use super::support::{Fixture, path};
use std::{fs, os::unix::fs::symlink};
use studiolo_vault_broker::{Mutation, VaultReader, VaultWriter};

#[test]
fn existing_history_must_be_exact_unlinked_bytes_before_replacement() {
    for damage in ["different-bytes", "symlink", "hardlink", "directory"] {
        let fixture = Fixture::initialized();
        let mut broker = fixture.open();
        let first = broker
            .mutate(Mutation::create(
                path("note.md"),
                b"original".to_vec(),
                "initial",
            ))
            .unwrap();
        let history = fixture
            .data
            .join("history")
            .join(first.revision.to_string());
        let outside = fixture.directory.path().join("outside-original");
        match damage {
            "different-bytes" => fs::write(&history, b"forged").unwrap(),
            "directory" => fs::create_dir(&history).unwrap(),
            _ => {
                fs::write(&outside, b"original").unwrap();
                if damage == "symlink" {
                    symlink(&outside, &history).unwrap();
                } else {
                    fs::hard_link(&outside, &history).unwrap();
                }
            }
        }
        assert!(
            broker
                .mutate(Mutation::replace(
                    path("note.md"),
                    b"replacement".to_vec(),
                    first.revision,
                    "replace"
                ))
                .is_err(),
            "accepted {damage} history"
        );
        assert_eq!(broker.read(&path("note.md")).unwrap().content, b"original");
        if outside.exists() {
            assert_eq!(fs::read(outside).unwrap(), b"original");
        }
    }
}
