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
}
