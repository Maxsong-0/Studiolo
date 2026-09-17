//! 固定资源上限属于公开失败语义；不向生产 Interface 添加故障开关。

use super::{
    legacy::Legacy,
    support::{Fixture, inventory, path},
};
use std::{fs, io::Read};
use studiolo_vault_broker::{BrokerError, FsVaultBroker, Mutation, VaultWriter};

const CONTENT_LIMIT: usize = 4 * 1024 * 1024;

#[test]
fn oversized_identity_binding_and_journal_are_refused_without_repair() {
    for record in ["identity", "root-binding", "data-binding", "journal"] {
        let fixture = Fixture::initialized();
        let (target, limit) = match record {
            "identity" => (fixture.marker(), 4096),
            "root-binding" => (fixture.vault.join(".workbench/broker-binding.json"), 4096),
            "data-binding" => (fixture.data.join("broker-binding.json"), 4096),
            _ => (fixture.data.join("writer-state.json"), 64 * 1024),
        };
        fs::write(target, vec![b' '; limit + 1]).unwrap();
        let before = inventory(fixture.directory.path());
        assert!(
            matches!(
                FsVaultBroker::open_existing(&fixture.vault, &fixture.data),
                Err(BrokerError::WriterLimitExceeded)
            ),
            "accepted oversized {record}"
        );
        assert!(
            matches!(
                FsVaultBroker::open(&fixture.vault, &fixture.data),
                Err(BrokerError::WriterLimitExceeded)
            ),
            "repaired oversized {record}"
        );
        assert_eq!(inventory(fixture.directory.path()), before);
    }
}

#[test]
fn oversized_legacy_replay_is_rejected_before_adoption() {
    let legacy = Legacy::new(true);
    fs::write(&legacy.record, vec![b' '; 64 * 1024 + 1]).unwrap();
    let before = inventory(legacy.fixture.directory.path());
    assert!(matches!(
        FsVaultBroker::open(&legacy.fixture.vault, &legacy.fixture.data),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert_eq!(inventory(legacy.fixture.directory.path()), before);
}

#[test]
fn mutation_content_and_key_budgets_reject_before_installation() {
    let fixture = Fixture::initialized();
    let mut broker = fixture.open();
    let before = inventory(&fixture.vault);
    for request in [
        Mutation::create(path("oversized.md"), vec![b'x'; CONTENT_LIMIT + 1], "large"),
        Mutation::create(path("long-key.md"), b"small".to_vec(), "k".repeat(4097)),
    ] {
        assert!(matches!(
            broker.mutate(request),
            Err(BrokerError::WriterLimitExceeded)
        ));
        assert_eq!(inventory(&fixture.vault), before);
        assert_eq!(broker.write_status().unwrap().committed_through, 0);
        assert!(!broker.write_status().unwrap().unresolved);
    }
    // 边界本身可提交，证明限制并非把正常输入一概拒绝。
    broker
        .mutate(Mutation::create(
            path("boundary.md"),
            vec![b'x'; CONTENT_LIMIT],
            "k".repeat(4096),
        ))
        .unwrap();
    assert_eq!(
        fs::metadata(fixture.vault.join("boundary.md"))
            .unwrap()
            .len(),
        CONTENT_LIMIT as u64
    );
    assert_eq!(broker.write_status().unwrap().committed_through, 1);
}

#[test]
fn oversized_actual_old_content_or_history_blocks_replacement() {
    for source in ["canonical", "history"] {
        let fixture = Fixture::initialized();
        let mut broker = fixture.open();
        let first = broker
            .mutate(Mutation::create(
                path("note.md"),
                b"original".to_vec(),
                "first",
            ))
            .unwrap();
        // 旧对象已由流式保全支持到256MiB；此项只验证新工程上限加一的先验拒绝。
        // 稀疏长度用于预算，真实大字节/哈希由独立流式契约覆盖。
        let target = if source == "canonical" {
            fixture.vault.join("note.md")
        } else {
            fixture
                .data
                .join("history")
                .join(first.revision.to_string())
        };
        fs::OpenOptions::new()
            .write(true)
            .open(&target)
            .unwrap()
            .set_len(256 * 1024 * 1024 + 1)
            .unwrap();
        let canonical = fixture.vault.join("note.md");
        let before = fs::metadata(&canonical).unwrap();
        let expected = first.revision;
        assert!(
            matches!(
                broker.mutate(Mutation::replace(
                    path("note.md"),
                    b"replacement".to_vec(),
                    expected,
                    "replace"
                )),
                Err(BrokerError::WriterLimitExceeded)
            ),
            "accepted oversized {source}"
        );
        let after = fs::metadata(&canonical).unwrap();
        assert_eq!(after.len(), before.len());
        assert_eq!(after.modified().unwrap(), before.modified().unwrap());
        let mut prefix = [0; 8];
        fs::File::open(canonical)
            .unwrap()
            .read_exact(&mut prefix)
            .unwrap();
        assert_eq!(&prefix, b"original");
        assert_eq!(fs::metadata(target).unwrap().len(), 256 * 1024 * 1024 + 1);
        assert_eq!(broker.write_status().unwrap().committed_through, 1);
    }
}

#[test]
fn legacy_adoption_has_a_bounded_number_of_replay_records() {
    let legacy = Legacy::new(false);
    let record = fs::read(&legacy.record).unwrap();
    for index in 0..4096 {
        let target = legacy
            .fixture
            .data
            .join("idempotency")
            .join(format!("{index:064x}.json"));
        fs::write(target, &record).unwrap();
    }
    assert!(matches!(
        FsVaultBroker::open(&legacy.fixture.vault, &legacy.fixture.data),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(!legacy.fixture.data.join("writer-state.json").exists());
    assert!(
        !legacy
            .fixture
            .vault
            .join(".workbench/broker-binding.json")
            .exists()
    );
}

#[test]
fn legacy_adoption_bounds_total_bytes_even_when_each_record_is_within_budget() {
    let legacy = Legacy::new(true);
    let mut record = fs::read(&legacy.record).unwrap();
    // 旧 M0 接受 JSON 尾部空白；采用后仍须保留原字节。
    record.resize(60 * 1024, b' ');
    fs::write(&legacy.record, &record).unwrap();
    let accepted = FsVaultBroker::open(&legacy.fixture.vault, &legacy.fixture.data).unwrap();
    assert_eq!(fs::read(&legacy.record).unwrap(), record);
    drop(accepted);

    let large = Legacy::new(true);
    let mut record = fs::read(&large.record).unwrap();
    record.resize(60 * 1024, b' ');
    fs::write(&large.record, &record).unwrap();
    for index in 0..273 {
        fs::write(
            large
                .fixture
                .data
                .join("idempotency")
                .join(format!("{index:064x}.json")),
            &record,
        )
        .unwrap();
    }
    // 274 × 60 KiB > 16 MiB，条数与单条大小仍分别低于对应上限。
    assert!(matches!(
        FsVaultBroker::open(&large.fixture.vault, &large.fixture.data),
        Err(BrokerError::WriterLimitExceeded)
    ));
    assert!(!large.fixture.data.join("writer-state.json").exists());
}
