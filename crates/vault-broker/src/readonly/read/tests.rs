//! Deterministic real-file checks at the shared stable-read boundary.

use cap_std::fs::File;
use std::{fs, io::Write as _};
use studiolo_core::VaultPath;

use super::{finish_read, read_with, replica_namespace};
use crate::BrokerError;

#[test]
fn growth_after_initial_metadata_cannot_bypass_the_byte_budget() {
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("note.md");
    fs::write(&selected, b"1234").unwrap();
    let file = File::from_std(fs::File::open(&selected).unwrap());
    let before = file.metadata().unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&selected)
        .unwrap()
        .write_all(b"56789")
        .unwrap();
    let logical = VaultPath::parse("note.md").unwrap();
    let result = finish_read(&logical, 4, replica_namespace(), file, before, || {
        Ok(File::from_std(fs::File::open(&selected).unwrap()))
    });
    assert!(
        matches!(result, Err(BrokerError::ReadLimitExceeded { path, max_bytes: 4 }) if path == logical)
    );
}

#[test]
fn growth_within_budget_is_still_an_unstable_read() {
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("note.md");
    fs::write(&selected, b"1234").unwrap();
    let file = File::from_std(fs::File::open(&selected).unwrap());
    let before = file.metadata().unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&selected)
        .unwrap()
        .write_all(b"5")
        .unwrap();
    let logical = VaultPath::parse("note.md").unwrap();
    let result = finish_read(&logical, 64, replica_namespace(), file, before, || {
        Ok(File::from_std(fs::File::open(&selected).unwrap()))
    });
    assert!(matches!(result, Err(BrokerError::UnstableRead(path)) if path == logical));
}

#[test]
fn replacing_the_file_before_final_reopen_rejects_the_old_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let selected = temp.path().join("note.md");
    let replacement = temp.path().join("replacement");
    fs::write(&selected, b"original").unwrap();
    fs::write(&replacement, b"replaced").unwrap();
    let logical = VaultPath::parse("note.md").unwrap();
    let mut opens = 0;
    let result = read_with(&logical, 64, replica_namespace(), || {
        opens += 1;
        if opens == 2 {
            fs::rename(&replacement, &selected).unwrap();
        }
        Ok(File::from_std(fs::File::open(&selected).unwrap()))
    });
    assert!(matches!(result, Err(BrokerError::UnstableRead(path)) if path == logical));
}
