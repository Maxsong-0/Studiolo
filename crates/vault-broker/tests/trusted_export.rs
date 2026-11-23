//! 导出扫描：尝试正文只跟清单第 2 版，历史正文跟着所属文档走，归不了类的整批拒绝。
//!
//! 这里的目录树是手搭的，只管分类规则；真实六 owner 导出走同一个扫描器的证据在
//! `local-server` 的总封存集成用例里（第 4 种模式）。
use sha2::{Digest, Sha256};
use std::{fs, path::Path};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef, TrustedExportClass, TrustedExportFact, VaultUuid,
};
use studiolo_vault_broker::read_trusted_export;

const ATTEMPT: &str = "sources/attempts/task/attempt";

fn provenance() -> ArtifactProvenance {
    ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
        DataLabels::new(
            Classification::Restricted,
            BackupScope::Full,
            DesktopSync::Deny,
            ModelEgress::Deny,
        )
        .unwrap(),
        4_000_000_000,
        "b".repeat(64),
    )
    .unwrap()
}

fn manifest(version: u32, with_provenance: bool, artifacts: &[&str]) -> Vec<u8> {
    let mut value = serde_json::json!({
        "format_version": version,
        "artifacts": artifacts
            .iter()
            .map(|path| serde_json::json!({ "path": path }))
            .collect::<Vec<_>>(),
    });
    if with_provenance {
        value["provenance"] = serde_json::to_value(provenance()).unwrap();
    }
    serde_json::to_vec(&value).unwrap()
}

fn write(root: &Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

fn digest(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 给一篇文档写历史目录（文件名是逻辑路径的摘要，和真实布局一样）和对应的历史正文。
/// 每个字节值代表一个 revision；同一个值出现两次就是「改回旧内容」。
fn history(root: &Path, document: &str, entries: &[u8], garbage: &[u8]) {
    let name = digest(document);
    let revision = |byte: &u8| vec![*byte; 32];
    write(
        root,
        &format!("writer-data/history/meta-v1/catalogs/{name}.json"),
        &serde_json::to_vec(&serde_json::json!({
            "version": 3,
            "pair_id": VaultUuid::new(),
            "path": document,
            "entries": entries
                .iter()
                .map(|byte| serde_json::json!({
                    "entry": { "id": "ab".repeat(16), "revision": revision(byte), "bytes": 3 }
                }))
                .collect::<Vec<_>>(),
            "garbage": garbage.iter().map(revision).collect::<Vec<_>>(),
        }))
        .unwrap(),
    );
    for byte in entries.iter().chain(garbage) {
        write(root, &body(*byte), b"old");
    }
}

/// 写一份已完成的批次终态记录（文件名是批次号的摘要，和真实布局一样），只带扫描用得到的字段。
/// 每项是（文档路径、原文、新文）；新建的那项没有原文。正文另外写。
fn batch(root: &Path, id: &str, items: &[(&str, Option<u8>, u8)]) {
    batch_in_phase(root, id, "completed", items);
}

fn batch_in_phase(root: &Path, id: &str, phase: &str, items: &[(&str, Option<u8>, u8)]) {
    let revision = |byte: u8| vec![byte; 32];
    write(
        root,
        &format!("writer-data/batch-{}.json", digest(id)),
        &serde_json::to_vec(&serde_json::json!({
            "pair": VaultUuid::new(),
            "active": {
                "batch_id": id,
                "phase": phase,
                "items": items
                    .iter()
                    .map(|(path, previous, target)| serde_json::json!({
                        "path": path,
                        "previous_revision": previous.map(revision),
                        "target_revision": revision(*target),
                    }))
                    .collect::<Vec<_>>(),
            },
        }))
        .unwrap(),
    );
}

fn body(byte: u8) -> String {
    let revision = format!("{byte:02x}").repeat(32);
    format!("writer-data/history/{revision}")
}

fn scan(root: &Path) -> Result<Vec<TrustedExportFact>, String> {
    read_trusted_export(root, VaultUuid::new()).map_err(|error| error.to_string())
}

fn fact<'a>(facts: &'a [TrustedExportFact], path: &str) -> &'a TrustedExportFact {
    facts
        .iter()
        .find(|fact| fact.export_path.as_str() == path)
        .unwrap_or_else(|| panic!("没有 {path} 的事实"))
}

#[test]
fn every_file_gets_exactly_one_fact_by_owner_directory_manifest_or_history_catalog() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    write(
        root,
        &format!("canonical/{ATTEMPT}/manifest.json"),
        // 清单列了 gone.md，但文件已经不在：不算错。
        &manifest(
            2,
            true,
            &[
                &format!("{ATTEMPT}/result.md"),
                &format!("{ATTEMPT}/nested/manifest.json"),
                &format!("{ATTEMPT}/gone.md"),
            ],
        ),
    );
    write(root, &format!("canonical/{ATTEMPT}/result.md"), b"body");
    // 更深处叫 manifest.json 的是产物，不是清单。
    write(
        root,
        &format!("canonical/{ATTEMPT}/nested/manifest.json"),
        b"an artifact",
    );
    write(root, "canonical/notes/plain.md", b"note");
    write(root, "canonical/.workbench/vault.json", b"{}");
    write(root, "control/control.sqlite3", b"control");
    write(root, "audit/audit.sqlite3", b"audit");
    write(root, "agent-audit/audit.sqlite3", b"agent audit");
    write(root, "writer-data/writer-state.json", b"{}");
    write(root, "writer-data/history/meta-v1/binding.json", b"{}");
    // 变更流分页和历史目录在同一个文件夹里，但不是历史目录。
    write(
        root,
        "writer-data/history/meta-v1/catalogs/feed-v1-0.json",
        b"{\"another\":\"schema\"}",
    );
    // 普通笔记：改回过旧内容（两条记录同一份正文），还有一份等待删除的旧正文。
    history(root, "notes/plain.md", &[0x11, 0x11], &[0x12]);
    // 已经删掉的笔记，历史还在。
    history(root, "notes/deleted.md", &[0x13], &[]);
    // 尝试正文的旧版本，包括清单列了但已经不在的那一个。
    history(root, &format!("{ATTEMPT}/result.md"), &[0x21], &[]);
    history(root, &format!("{ATTEMPT}/gone.md"), &[0x22], &[]);

    let facts = scan(root).unwrap();
    let on_disk = walkdir(root);
    assert_eq!(facts.len(), on_disk, "每个文件恰好一条事实");

    for path in [
        format!("canonical/{ATTEMPT}/manifest.json"),
        format!("canonical/{ATTEMPT}/result.md"),
        format!("canonical/{ATTEMPT}/nested/manifest.json"),
        // 来源资料的旧版本仍是来源资料，带着同一份原批准（期限跟着走）。
        body(0x21),
        body(0x22),
    ] {
        let fact = fact(&facts, &path);
        assert_eq!(fact.class, TrustedExportClass::SourceMaterial, "{path}");
        assert_eq!(fact.provenance, Some(provenance()), "{path}");
    }
    for (path, class) in [
        ("canonical/notes/plain.md", TrustedExportClass::Canonical),
        (
            "canonical/.workbench/vault.json",
            TrustedExportClass::Canonical,
        ),
        (
            "control/control.sqlite3",
            TrustedExportClass::ControlDatabase,
        ),
        ("audit/audit.sqlite3", TrustedExportClass::AuditLedger),
        ("agent-audit/audit.sqlite3", TrustedExportClass::AuditLedger),
        (
            "writer-data/writer-state.json",
            TrustedExportClass::Canonical,
        ),
        (
            "writer-data/history/meta-v1/catalogs/feed-v1-0.json",
            TrustedExportClass::Canonical,
        ),
        (body(0x11).as_str(), TrustedExportClass::FullScopeL0),
        (body(0x12).as_str(), TrustedExportClass::FullScopeL0),
        (body(0x13).as_str(), TrustedExportClass::FullScopeL0),
    ] {
        let fact = fact(&facts, path);
        assert_eq!(fact.class, class, "{path}");
        assert!(fact.provenance.is_none(), "{path}");
    }
}

/// 2026-11-13 以前的批次只在终态记录里登记过每项的原文和新文（回滚要读原文），历史目录里没有（BUG-052）。
/// 这两份正文跟着该项的文档走，和目录认领的一样：普通笔记的是完整范围 L0，尝试正文的带原批准。
#[test]
fn a_batch_record_claims_the_old_and_new_bodies_of_its_items_for_their_documents() {
    let root = tempfile::tempdir().unwrap();
    let root = root.path();
    let result = format!("{ATTEMPT}/result.md");
    write(
        root,
        &format!("canonical/{ATTEMPT}/manifest.json"),
        &manifest(2, true, &[&result]),
    );
    // 原文在目录里、新文只有批次认领（169 上 1 批是这样）。
    history(root, "notes/plain.md", &[0x41], &[]);
    batch(root, "first", &[("notes/plain.md", Some(0x41), 0x42)]);
    // 原文、新文都只有批次认领（另外 10 批）；新建的那项没有原文。
    batch(
        root,
        "second",
        &[
            ("notes/other.md", Some(0x43), 0x44),
            (result.as_str(), None, 0x45),
        ],
    );
    for byte in [0x42, 0x43, 0x44, 0x45] {
        write(root, &body(byte), b"old");
    }
    // 只有写者数据根这一层的文件才是批次记录；子目录里的（目录名也以 batch- 开头）照普通写者状态归类。
    write(root, "writer-data/batch-old/batch-x.json", b"not a record");

    let facts = scan(root).unwrap();
    assert_eq!(facts.len(), walkdir(root), "每个文件恰好一条事实");
    for byte in [0x41, 0x42, 0x43, 0x44] {
        let fact = fact(&facts, &body(byte));
        assert_eq!(fact.class, TrustedExportClass::FullScopeL0, "{byte:#x}");
        assert!(fact.provenance.is_none(), "{byte:#x}");
    }
    let material = fact(&facts, &body(0x45));
    assert_eq!(material.class, TrustedExportClass::SourceMaterial);
    assert_eq!(material.provenance, Some(provenance()));
    for path in [
        format!("writer-data/batch-{}.json", digest("first")),
        format!("writer-data/batch-{}.json", digest("second")),
        "writer-data/batch-old/batch-x.json".to_owned(),
    ] {
        let fact = fact(&facts, &path);
        assert_eq!(fact.class, TrustedExportClass::Canonical, "{path}");
        assert!(fact.provenance.is_none(), "{path}");
    }
}

fn walkdir(directory: &Path) -> usize {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .map(|path| if path.is_dir() { walkdir(&path) } else { 1 })
        .sum()
}

#[test]
fn anything_that_cannot_be_classified_rejects_the_whole_export() {
    let reject = |build: &dyn Fn(&Path), expected: &str| {
        let root = tempfile::tempdir().unwrap();
        build(root.path());
        let error = scan(root.path()).unwrap_err();
        assert!(
            error.contains(expected),
            "期望「{expected}」，得到「{error}」"
        );
    };
    let manifest_path = format!("canonical/{ATTEMPT}/manifest.json");
    let result = format!("{ATTEMPT}/result.md");

    // 旧清单（第 1 版，或者第 2 版却没有原批准）不升成完整范围。
    reject(
        &|root| {
            write(root, &manifest_path, &manifest(1, false, &[&result]));
            write(root, &format!("canonical/{result}"), b"body");
        },
        "不能升为完整范围",
    );
    reject(
        &|root| write(root, &manifest_path, &manifest(2, false, &[])),
        "不能升为完整范围",
    );
    // 尝试目录里有清单没列出的文件，或者干脆没有清单。
    reject(
        &|root| {
            write(root, &manifest_path, &manifest(2, true, &[&result]));
            write(root, &format!("canonical/{result}"), b"body");
            write(root, &format!("canonical/{ATTEMPT}/extra.md"), b"extra");
        },
        "没有被本目录的清单列出",
    );
    reject(
        &|root| write(root, &format!("canonical/{result}"), b"body"),
        "没有被本目录的清单列出",
    );
    // 清单认领别处的文件：不能把必含的普通笔记说成会到期的来源资料。
    reject(
        &|root| {
            write(
                root,
                &manifest_path,
                &manifest(2, true, &["notes/plain.md"]),
            );
            write(root, "canonical/notes/plain.md", b"note");
        },
        "清单列了本目录以外的文件",
    );
    reject(
        &|root| {
            write(
                root,
                &manifest_path,
                &manifest(2, true, &["sources/attempts/task/other/result.md"]),
            );
        },
        "清单列了本目录以外的文件",
    );
    reject(
        &|root| {
            write(
                root,
                &manifest_path,
                &manifest(2, true, &[&result, &result]),
            );
            write(root, &format!("canonical/{result}"), b"body");
        },
        "认领了两次",
    );
    // 历史：没人认领的正文、尝试正文的历史却找不到清单、目录文件名和内容对不上。
    reject(
        &|root| write(root, &body(0x31), b"orphan"),
        "没有目录或批次记录认领",
    );
    reject(&|root| history(root, &result, &[0x32], &[]), "找不到原清单");
    reject(
        &|root| {
            history(root, "notes/a.md", &[0x33], &[]);
            let catalogs = root.join("writer-data/history/meta-v1/catalogs");
            let only = fs::read_dir(&catalogs).unwrap().next().unwrap().unwrap();
            fs::rename(
                only.path(),
                catalogs.join(format!("{}.json", "c".repeat(64))),
            )
            .unwrap();
        },
        "文件名对不上",
    );
    reject(
        &|root| {
            history(root, "notes/a.md", &[0x34], &[]);
            history(root, "notes/b.md", &[0x34], &[]);
        },
        "两篇文档认领",
    );
    // 批次记录：读不出来、文件名不是批次号的摘要（写者自己也这样核）、不是完成的终态（写者读到也报坏）、
    // 认领了另一篇文档目录里的正文。
    reject(
        &|root| {
            write(
                root,
                &format!("writer-data/batch-{}.json", digest("x")),
                b"{",
            )
        },
        "批次记录读不出来",
    );
    reject(
        &|root| {
            batch(root, "moved", &[("notes/a.md", None, 0x35)]);
            write(root, &body(0x35), b"new");
            fs::rename(
                root.join(format!("writer-data/batch-{}.json", digest("moved"))),
                root.join(format!("writer-data/batch-{}.json", digest("other"))),
            )
            .unwrap();
        },
        "批次记录的文件名对不上",
    );
    reject(
        &|root| {
            batch(root, "odd", &[("notes/a.md", None, 0x36)]);
            write(root, &body(0x36), b"new");
            fs::rename(
                root.join(format!("writer-data/batch-{}.json", digest("odd"))),
                root.join("writer-data/batch-odd.json"),
            )
            .unwrap();
        },
        "批次记录的文件名对不上",
    );
    reject(
        &|root| {
            batch_in_phase(root, "half", "installing", &[("notes/a.md", None, 0x38)]);
            write(root, &body(0x38), b"new");
        },
        "批次记录不是完成的终态",
    );
    reject(
        &|root| {
            history(root, "notes/a.md", &[0x37], &[]);
            batch(root, "elsewhere", &[("notes/b.md", None, 0x37)]);
        },
        "两篇文档认领",
    );
    // 崩溃时写到一半留下的临时文件，不管是谁的；名字像暂存正文、却不是 Broker 会写出来的样子。
    reject(
        &|root| {
            write(
                root,
                "writer-data/.artifact-candidate-ab.json.studiolo-tmp-7-1",
                b"half a staged body",
            )
        },
        "写到一半留下的临时文件",
    );
    reject(
        &|root| {
            write(
                root,
                &format!("writer-data/history/.{}.studiolo-tmp-7-2", "ab".repeat(32)),
                b"half an old version",
            )
        },
        "写到一半留下的临时文件",
    );
    reject(
        &|root| {
            write(
                root,
                "canonical/notes/.plain.md.studiolo-tmp-7-3",
                b"half a note",
            )
        },
        "写到一半留下的临时文件",
    );
    reject(
        &|root| write(root, "writer-data/artifact-candidate-ab.json.bak", b"{}"),
        "暂存正文没有原批准",
    );
    reject(
        &|root| write(root, "writer-data/nested/artifact-candidate-ab.json", b"{}"),
        "暂存正文不在写者数据根这一层",
    );
    // 不认识的顶层目录、不是普通文件的东西。
    reject(
        &|root| write(root, "mystery/secret.txt", b"no owner"),
        "不认识的顶层目录",
    );
    reject(
        &|root| {
            write(root, "canonical/notes/plain.md", b"note");
            std::os::unix::fs::symlink("plain.md", root.join("canonical/notes/link.md")).unwrap();
        },
        "不是普通文件",
    );
}
