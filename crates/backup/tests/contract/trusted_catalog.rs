//! 来源事实组装：没有原批准的尝试正文不能进入目录。
use studiolo_backup::checkpoint::{inclusion::SourceCategory, staging::trusted_source_catalog};
use studiolo_core::{
    ArtifactProvenance, BackupScope, Classification, DataLabels, DesktopSync, ModelEgress,
    Revision, SourcePolicyRef, TrustedExportClass, TrustedExportFact, VaultPath, VaultUuid,
};

fn labels() -> DataLabels {
    DataLabels::new(
        Classification::Restricted,
        BackupScope::Full,
        DesktopSync::Deny,
        ModelEgress::Deny,
    )
    .unwrap()
}

#[test]
fn source_material_without_provenance_is_rejected_and_a_granted_fact_is_kept() {
    let vault = VaultUuid::new();
    let plain = VaultPath::parse("canonical/notes/plain.md").unwrap();
    assert!(
        trusted_source_catalog(
            vault,
            [TrustedExportFact {
                export_path: plain.clone(),
                class: TrustedExportClass::SourceMaterial,
                provenance: None,
                revision: Revision::compute(vault, &plain, b"note"),
            }]
        )
        .is_err()
    );
    let provenance = ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
        labels(),
        4_000_000_000,
        "b".repeat(64),
    )
    .unwrap();
    let path = VaultPath::parse("canonical/sources/attempts/task/attempt/result.md").unwrap();
    let catalog = trusted_source_catalog(
        vault,
        [TrustedExportFact {
            export_path: path.clone(),
            class: TrustedExportClass::SourceMaterial,
            provenance: Some(provenance),
            revision: Revision::compute(vault, &path, b"body"),
        }],
    )
    .unwrap();
    let record = catalog.registration(&path).unwrap();
    assert_eq!(record.source.category, SourceCategory::SourceMaterial);
    assert_eq!(record.source.labels.clone().unwrap(), labels());
    // 期限跟着原批准走：有了它，到期后这份正文才会只留非正文记录。
    assert_eq!(
        record.source.expires_at.unwrap().millis(),
        4_000_000_000_000
    );
}

#[test]
fn only_source_material_may_carry_an_approval() {
    let vault = VaultUuid::new();
    let provenance = ArtifactProvenance::new(
        SourcePolicyRef::new("fixture-source", "a".repeat(64)).unwrap(),
        labels(),
        4_000_000_000,
        "b".repeat(64),
    )
    .unwrap();
    let blob = VaultPath::parse(format!("writer-data/history/{}", "11".repeat(32))).unwrap();
    // 必含类别不看期限；带期限的内容以这些类别进来，期限就失效了，所以整批拒绝。
    for class in [
        TrustedExportClass::FullScopeL0,
        TrustedExportClass::Canonical,
        TrustedExportClass::ControlDatabase,
        TrustedExportClass::AuditLedger,
    ] {
        assert!(
            trusted_source_catalog(
                vault,
                [TrustedExportFact {
                    export_path: blob.clone(),
                    class,
                    provenance: Some(provenance.clone()),
                    revision: Revision::compute(vault, &blob, b"old"),
                }]
            )
            .is_err(),
            "{class:?}"
        );
    }
    let kept = trusted_source_catalog(
        vault,
        [TrustedExportFact {
            export_path: blob.clone(),
            class: TrustedExportClass::FullScopeL0,
            provenance: None,
            revision: Revision::compute(vault, &blob, b"old"),
        }],
    )
    .unwrap();
    let record = kept.registration(&blob).unwrap();
    assert_eq!(record.source.category, SourceCategory::FullScopeL0);
    assert!(record.source.expires_at.is_none() && record.source.source_policy.is_none());
}
