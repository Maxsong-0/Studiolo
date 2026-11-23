//! 2026-11-13 以前的批次提交只把每项的原文、新文写进 `history/`、记在批次终态记录里，
//! 不进历史目录（BUG-052）。169 上 9 月留下 11 批这样的数据，导出扫描当它们没有认领，整批封存失败。
//! 这里用真写者提交批次，再把历史目录里由批次补上的条目去掉，还原那时的布局。
use super::path;
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use studiolo_core::TrustedExportClass;
use studiolo_vault_broker::{
    BatchChange, BatchEdit, BatchRequest, BatchWriter, FsVaultBroker, Mutation, TextPatch,
    VaultReader, VaultWriter, read_trusted_export,
};

const DOCUMENT: &str = "notes/plain.md";

fn patch(broker: &mut FsVaultBroker, id: &str, before: &str, after: &str) {
    let expected = broker.read(&path(DOCUMENT)).unwrap().revision;
    let request = BatchRequest {
        batch_id: id.into(),
        changes: vec![BatchChange {
            path: path(DOCUMENT),
            edit: BatchEdit::Patch {
                expected,
                patches: vec![TextPatch {
                    before: before.into(),
                    after: after.into(),
                }],
            },
        }],
    };
    let preview = broker.preview_batch(&request).unwrap();
    broker.commit_batch(request, &preview).unwrap();
}

/// 去掉这篇文档历史目录里由批次补上的条目（9 月那版批次提交根本不写），返回去掉几条。
fn forget_batch_entries(data: &Path) -> usize {
    let digest: String = Sha256::digest(DOCUMENT.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let file = data.join(format!("history/meta-v1/catalogs/{digest}.json"));
    let mut catalog: serde_json::Value = serde_json::from_slice(&fs::read(&file).unwrap()).unwrap();
    let entries = catalog["entries"].as_array_mut().unwrap();
    let before = entries.len();
    entries.retain(|item| item.get("batch").is_none());
    let removed = before - entries.len();
    fs::write(&file, serde_json::to_vec(&catalog).unwrap()).unwrap();
    removed
}

#[test]
fn bodies_of_a_batch_committed_before_batch_history_existed_scan_by_their_batch_record() {
    let export = tempfile::tempdir().unwrap();
    let export = export.path().canonicalize().unwrap();
    let data = export.join("writer-data");
    let mut broker = FsVaultBroker::open(&export.join("canonical"), &data).unwrap();
    // 文档先经单文件写入建出来（这一版在目录里），再被两批改：第一批的原文在目录里；
    // 第二批的原文是第一批的新文。去掉批次条目后两份新文都只有批次认领，是 169 上那两种样子。
    broker
        .mutate(Mutation::create(path(DOCUMENT), b"one\n".to_vec(), "seed"))
        .unwrap();
    patch(&mut broker, "first", "one", "two");
    patch(&mut broker, "second", "two", "three");
    read_trusted_export(&export, broker.vault_uuid()).expect("现行代码写出的批次本来就扫得过");
    assert_eq!(forget_batch_entries(&data), 2, "两批的新文各一条");

    let facts = read_trusted_export(&export, broker.vault_uuid()).unwrap();
    let bodies: Vec<_> = facts
        .iter()
        .filter(|fact| {
            fact.export_path
                .as_str()
                .strip_prefix("writer-data/history/")
                .is_some_and(|rest| !rest.starts_with("meta-v1/"))
        })
        .collect();
    assert_eq!(bodies.len(), 3, "one、two、three 三份正文");
    for fact in bodies {
        assert_eq!(fact.class, TrustedExportClass::FullScopeL0, "{fact:?}");
        assert!(fact.provenance.is_none(), "{fact:?}");
    }
}
