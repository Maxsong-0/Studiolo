//! 导出扫描：尝试正文只跟清单第 2 版，L0 只继承已登记路径。
use std::fs;
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    SourcePolicyRef, TrustedExportClass, VaultUuid,
};
use studiolo_vault_broker::read_trusted_export;

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

fn manifest(version: u32, with_provenance: bool) -> String {
    let mut value = serde_json::json!({
        "format_version": version,
        "artifacts": [{"path": "sources/attempts/task/attempt/result.md"}]
    });
    if with_provenance {
        value["provenance"] = serde_json::to_value(provenance()).unwrap();
    }
    serde_json::to_string(&value).unwrap()
}

fn write(root: &std::path::Path, relative: &str, bytes: &[u8]) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, bytes).unwrap();
}

#[test]
fn attempt_files_keep_manifest_provenance_and_l0_inherits_the_note() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "canonical/sources/attempts/task/attempt/manifest.json",
        manifest(2, true).as_bytes(),
    );
    write(
        root.path(),
        "canonical/sources/attempts/task/attempt/result.md",
        b"attempt body",
    );
    write(root.path(), "canonical/notes/plain.md", b"note");
    write(root.path(), "control/control.sqlite3", b"control");
    let revision = "11".repeat(32);
    write(
        root.path(),
        &format!("writer-data/history/{revision}"),
        b"old note",
    );
    write(
        root.path(),
        "writer-data/history/meta-v1/catalogs/note.json",
        serde_json::to_string(&serde_json::json!({
            "version": 1,
            "pair_id": VaultUuid::new(),
            "path": "notes/plain.md",
            "entries": [{"entry": {"revision": vec![0x11; 32]}}],
            "garbage": [],
            "feed_retire": []
        }))
        .unwrap()
        .as_bytes(),
    );
    let facts = read_trusted_export(root.path(), VaultUuid::new()).unwrap();
    let class = |path: &str| {
        facts
            .iter()
            .find(|fact| fact.export_path.as_str() == path)
            .unwrap()
            .class
    };
    assert_eq!(
        class("canonical/sources/attempts/task/attempt/result.md"),
        TrustedExportClass::SourceMaterial
    );
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact.export_path.as_str()
                == "canonical/sources/attempts/task/attempt/result.md")
            .unwrap()
            .provenance
            .as_ref()
            .unwrap()
            .source_policy()
            .source_id(),
        "fixture-source"
    );
    assert_eq!(
        class("control/control.sqlite3"),
        TrustedExportClass::ControlDatabase
    );
    assert_eq!(
        class(&format!("writer-data/history/{revision}")),
        TrustedExportClass::FullScopeL0
    );
    assert_eq!(
        class("canonical/notes/plain.md"),
        TrustedExportClass::Canonical
    );
}

#[test]
fn old_attempt_manifest_unknown_directory_and_orphan_blob_fail() {
    let vault = VaultUuid::new();
    let missing = tempfile::tempdir().unwrap();
    write(
        missing.path(),
        "canonical/sources/attempts/task/attempt/manifest.json",
        manifest(1, false).as_bytes(),
    );
    write(
        missing.path(),
        "canonical/sources/attempts/task/attempt/result.md",
        b"body",
    );
    let error = read_trusted_export(missing.path(), vault).unwrap_err();
    assert!(error.to_string().contains("不能升为完整范围"), "{error}");

    let unknown = tempfile::tempdir().unwrap();
    write(unknown.path(), "mystery/secret.txt", b"no owner");
    assert!(read_trusted_export(unknown.path(), vault).is_err());

    let orphan = tempfile::tempdir().unwrap();
    write(
        orphan.path(),
        "writer-data/history/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        b"orphan",
    );
    assert!(read_trusted_export(orphan.path(), vault).is_err());
}
