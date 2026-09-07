//! A writer lease does not weaken create-only installation against outside changes.

#![cfg(target_os = "linux")]

use std::{
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    sync::{Arc, Barrier},
    thread,
};
use studiolo_core::VaultPath;
use studiolo_vault_broker::{FsVaultBroker, Mutation, VaultWriter as _};
use tempfile::TempDir;

#[test]
fn create_only_does_not_replace_a_racing_external_create() {
    for _ in 0..16 {
        let vault = TempDir::new().unwrap();
        let data = TempDir::new().unwrap();
        let mut broker = FsVaultBroker::open(vault.path(), data.path()).unwrap();
        let target = vault.path().join("contended.md");
        let barrier = Arc::new(Barrier::new(2));
        let ready = barrier.clone();
        let outside_path = target.clone();
        let outside = thread::spawn(move || {
            ready.wait();
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(outside_path)
            {
                Ok(mut file) => {
                    file.write_all(b"outside content").unwrap();
                    file.sync_all().unwrap();
                    true
                }
                Err(error) if error.kind() == ErrorKind::AlreadyExists => false,
                Err(error) => panic!("unexpected external-create error: {error}"),
            }
        });
        barrier.wait();
        let own = broker.mutate(Mutation::create(
            VaultPath::parse("contended.md").unwrap(),
            b"canonical content".to_vec(),
            "create-once",
        ));
        let external_won = outside.join().unwrap();
        assert_ne!(own.is_ok(), external_won, "both creates claimed success");
        assert_eq!(
            fs::read(target).unwrap(),
            if external_won {
                b"outside content".as_slice()
            } else {
                b"canonical content".as_slice()
            }
        );
    }
}
