//! 查重、人工正文保留、日期与行数只在临时目录/内存替身验证。
use super::*;
use crate::{BatchWriter, FsVaultBroker, MemoryVaultBroker, Mutation, VaultReader, VaultWriter};
fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}
fn request(id: &str) -> MemoryEntryRequest {
    MemoryEntryRequest {
        batch_id: id.into(),
        scope: path("项目/Studiolo/03-记忆"),
        kind: "Decision".into(),
        summary: "Café Straße".into(),
        date: "2026-09-20".into(),
        body: "一条已证实的事实。".into(),
    }
}
struct Fixture {
    _temp: tempfile::TempDir,
    vault: std::path::PathBuf,
    data: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temp =
            tempfile::tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap()).unwrap();
        Self {
            vault: temp.path().join("vault"),
            data: temp.path().join("data"),
            _temp: temp,
        }
    }
    fn open(&self) -> FsVaultBroker {
        FsVaultBroker::open(&self.vault, &self.data).unwrap()
    }
}
fn dedupe_contract(broker: &mut (impl MemoryWriter + BatchWriter + VaultReader + VaultWriter)) {
    let original = request("first");
    let MemoryPreview::Create {
        request: batch,
        preview,
    } = broker.preview_memory(&original).unwrap()
    else {
        panic!("must create")
    };
    let target = batch.changes[0].path.clone();
    assert!(matches!(
        broker.read(&target),
        Err(BrokerError::NotFound(_))
    ));
    broker.commit_batch(batch, &preview).unwrap();
    let created = broker.read(&target).unwrap();
    let mut equivalent = request("second");
    equivalent.kind = " decision ".into();
    equivalent.summary = " CAFE\u{301} STRASSE ".into();
    equivalent.body = "不可覆盖已有正文".into();
    let MemoryPreview::Existing {
        path,
        revision,
        sha256,
        requires_patch,
    } = broker.preview_memory(&equivalent).unwrap()
    else {
        panic!("must dedupe")
    };
    assert_eq!(path, target);
    assert_eq!(revision, created.revision);
    assert_eq!(sha256, crate::sha256(&created.content));
    assert!(requires_patch);
    assert_eq!(broker.read(&target).unwrap().content, created.content);
    let mut different = equivalent;
    different.summary = "另一件事实".into();
    let MemoryPreview::Create { request, .. } = broker.preview_memory(&different).unwrap() else {
        panic!("different summary must create")
    };
    assert_ne!(request.changes[0].path, target);
}
#[test]
fn normalized_type_and_summary_dedupe_in_both_adapters() {
    dedupe_contract(&mut MemoryVaultBroker::new());
    dedupe_contract(&mut Fixture::new().open());
}
fn human_contract(broker: &mut (impl MemoryWriter + VaultReader + VaultWriter)) {
    let existing = path("项目/Studiolo/03-记忆/人工记录.md");
    let bytes = b"---\ntype: DECISION\nsummary: Cafe\xcc\x81 STRASSE\n---\nHuman original body\n";
    let receipt = broker
        .mutate(Mutation::create(existing.clone(), bytes.to_vec(), "human"))
        .unwrap();
    let MemoryPreview::Existing {
        path,
        revision,
        requires_patch,
        ..
    } = broker.preview_memory(&request("candidate")).unwrap()
    else {
        panic!("must preserve human")
    };
    assert_eq!(path, existing);
    assert_eq!(revision, receipt.revision);
    assert!(requires_patch);
    assert_eq!(broker.read(&existing).unwrap().content, bytes);
}
#[test]
fn existing_human_filename_needs_patch_and_is_not_duplicated() {
    human_contract(&mut MemoryVaultBroker::new());
    human_contract(&mut Fixture::new().open());
}
#[test]
fn absolute_dates_and_scope_are_strict() {
    let broker = MemoryVaultBroker::new();
    for date in [
        "today",
        "2026-02-29",
        "2026-9-20",
        "2026-09-20T00:00:00",
        "昨天",
    ] {
        let mut r = request("date");
        r.date = date.into();
        assert!(broker.preview_memory(&r).is_err(), "{date}");
    }
    let mut r = request("leap");
    r.date = "2024-02-29".into();
    assert!(broker.preview_memory(&r).is_ok());
    for scope in [
        "03-记忆外",
        "项目/Studiolo/02-设计",
        "sources/attempts",
        ".workbench/03-记忆",
    ] {
        let mut r = request("scope");
        r.scope = path(scope);
        assert!(broker.preview_memory(&r).is_err(), "{scope}");
    }
    assert!(VaultPath::parse("项目/Studiolo/03-记忆/../../escape").is_err());
}
#[test]
fn line_budget_counts_frontmatter_and_reports_recommendation() {
    let broker = MemoryVaultBroker::new();
    let mut r = request("line-limit");
    r.body = "事实\n".repeat(194);
    assert!(!broker.preview_memory(&r).unwrap().exceeds_target_lines());
    r.body = "事实\n".repeat(195);
    assert!(broker.preview_memory(&r).unwrap().exceeds_target_lines());
    r.body = "事实\n".repeat(294);
    assert!(broker.preview_memory(&r).is_ok());
    r.body = "事实\n".repeat(295);
    assert!(matches!(
        broker.preview_memory(&r),
        Err(BrokerError::WriterLimitExceeded)
    ));
}
#[test]
fn malformed_or_duplicate_existing_memories_fail_closed() {
    let mut broker = MemoryVaultBroker::new();
    for (index, name) in ["a.md", "b.md"].iter().enumerate() {
        broker
            .mutate(Mutation::create(
                path(&format!("项目/Studiolo/03-记忆/{name}")),
                b"---\ntype: decision\nsummary: 'Caf\xc3\xa9 Stra\xc3\x9fe'\n---\nbody".to_vec(),
                format!("seed-{index}"),
            ))
            .unwrap();
    }
    assert!(broker.preview_memory(&request("duplicate")).is_err());
}
#[test]
fn full_casefold_covers_non_lowercase_equivalence() {
    assert_eq!(fold::normalize("Straße"), fold::normalize("STRASSE"));
    assert_eq!(fold::normalize("ΟΣ"), fold::normalize("ος"));
    assert_eq!(fold::normalize("ﬃ"), fold::normalize("FFI"));
    assert_eq!(fold::normalize("Ꭰ"), fold::normalize("ꭰ"));
}

fn generic_batch_guard(broker: &mut (impl MemoryWriter + BatchWriter + VaultReader + VaultWriter)) {
    let original = request("guard-first");
    let MemoryPreview::Create {
        request: batch,
        preview,
    } = broker.preview_memory(&original).unwrap()
    else {
        panic!("must create")
    };
    let original_path = batch.changes[0].path.clone();
    broker.commit_batch(batch.clone(), &preview).unwrap();
    let mut duplicate = batch;
    duplicate.batch_id = "guard-duplicate".into();
    duplicate.changes[0].path = path("项目/Studiolo/03-记忆/other-name.md");
    assert!(broker.preview_batch(&duplicate).is_err());
    assert!(matches!(
        broker.read(&duplicate.changes[0].path),
        Err(BrokerError::NotFound(_))
    ));
    let old = broker.read(&original_path).unwrap();
    let mut patch = crate::BatchRequest {
        batch_id: "patch-date".into(),
        changes: vec![crate::BatchChange {
            path: original_path.clone(),
            edit: crate::BatchEdit::Patch {
                expected: old.revision,
                patches: vec![crate::TextPatch {
                    before: "2026-09-20".into(),
                    after: "today".into(),
                }],
            },
        }],
    };
    assert!(broker.preview_batch(&patch).is_err());
    patch.changes[0].edit = crate::BatchEdit::Patch {
        expected: old.revision,
        patches: vec![crate::TextPatch {
            before: "一条已证实的事实。".into(),
            after: "对同一事实的精确补充。".into(),
        }],
    };
    let preview = broker.preview_batch(&patch).unwrap();
    broker.commit_batch(patch, &preview).unwrap();
}
#[test]
fn generic_batch_cannot_bypass_memory_metadata_or_dedupe() {
    generic_batch_guard(&mut MemoryVaultBroker::new());
    generic_batch_guard(&mut Fixture::new().open());
}
fn dedupe_race(broker: &mut (impl MemoryWriter + BatchWriter + VaultReader + VaultWriter)) {
    let MemoryPreview::Create {
        request: batch,
        preview,
    } = broker.preview_memory(&request("race")).unwrap()
    else {
        panic!("must create")
    };
    broker
        .mutate(Mutation::create(
            path("项目/Studiolo/03-记忆/human-arrived.md"),
            b"---\ntype: decision\nsummary: Caf\xc3\xa9 Stra\xc3\x9fe\n---\nHuman".to_vec(),
            "race-human",
        ))
        .unwrap();
    assert!(broker.commit_batch(batch.clone(), &preview).is_err());
    assert!(matches!(
        broker.read(&batch.changes[0].path),
        Err(BrokerError::NotFound(_))
    ));
}
#[test]
fn final_commit_rescans_memory_after_earlier_preview() {
    dedupe_race(&mut MemoryVaultBroker::new());
    dedupe_race(&mut Fixture::new().open());
}
