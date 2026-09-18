//! inclusion plan 契约：先证明 ephemeral 不在源根内；元数据缺一项整批拒绝，
//! 绝不按文件名猜；必含类别不可排除；有排除项就必须走专用过滤 staging。

use std::path::{Path, PathBuf};
use studiolo_backup::checkpoint::inclusion::{
    Disposition, InclusionError, InclusionPlan, InclusionRequest, RejectionReason, SourceCategory,
    SourceRecord,
};
use studiolo_backup::checkpoint::types::{
    SealError, StagingMaterialization, TransferSource, TransferSourceKind,
};
use studiolo_core::{
    BackupScope, Classification, DataLabels, DesktopSync, ModelEgress, Revision, VaultPath,
    VaultUuid, checkpoint::InstantMillis,
};

fn labels(scope: BackupScope, sync: DesktopSync) -> DataLabels {
    DataLabels::new(
        Classification::Internal,
        scope,
        sync,
        ModelEgress::LocalOnly,
    )
    .expect("labels")
}

fn record(path: &str, category: SourceCategory, scope: BackupScope) -> SourceRecord {
    let path = VaultPath::parse(path).expect("path");
    let revision = Revision::compute(VaultUuid::new(), &path, b"bytes");
    SourceRecord {
        path,
        category,
        labels: Some(labels(
            scope,
            if scope == BackupScope::None {
                DesktopSync::Deny
            } else {
                DesktopSync::SummaryOnly
            },
        )),
        expires_at: Some(InstantMillis::from_millis(10_000)),
        crypto_erased: false,
        declared_revision: Some(revision),
        observed_revision: Some(revision),
    }
}

fn request(records: Vec<SourceRecord>) -> InclusionRequest {
    InclusionRequest {
        source_roots: vec![PathBuf::from("/srv/vault"), PathBuf::from("/srv/data")],
        ephemeral_root: PathBuf::from("/run/studiolo/ephemeral"),
        records,
        now: InstantMillis::from_millis(1_000),
    }
}

#[test]
fn a_plan_with_only_full_objects_needs_no_filter_staging() {
    let plan = InclusionPlan::build(&request(vec![
        record("notes/a.md", SourceCategory::Canonical, BackupScope::Full),
        record(
            "control.sqlite3",
            SourceCategory::ControlDatabase,
            BackupScope::Full,
        ),
    ]))
    .expect("plan");

    assert!(!plan.requires_filter_staging);
    assert!(
        plan.entries
            .iter()
            .all(|entry| entry.disposition == Disposition::FullObject)
    );
}

#[test]
fn an_ephemeral_root_inside_a_source_root_stops_the_plan_before_anything_else() {
    let mut inputs = request(vec![record(
        "notes/a.md",
        SourceCategory::Canonical,
        BackupScope::Full,
    )]);
    inputs.ephemeral_root = PathBuf::from("/srv/vault/.workbench/ephemeral");

    assert_eq!(
        InclusionPlan::build(&inputs),
        Err(InclusionError::EphemeralInsideSourceRoot)
    );
}

#[test]
fn missing_classification_or_backup_scope_rejects_the_whole_batch_by_name_agnostic_rules() {
    let mut records = vec![
        record("notes/a.md", SourceCategory::Canonical, BackupScope::Full),
        record(
            "notes/b.md",
            SourceCategory::SourceMaterial,
            BackupScope::Full,
        ),
    ];
    // 名字看起来像 canonical，但登记里没有标签：不许按文件名猜。
    records[1].labels = None;

    let error = InclusionPlan::build(&request(records));

    assert!(matches!(
        error,
        Err(InclusionError::BatchRejected {
            reason: RejectionReason::MissingClassificationOrScope,
            ..
        })
    ));
}

#[test]
fn missing_retention_metadata_rejects_the_whole_batch() {
    let mut records = vec![record(
        "notes/b.md",
        SourceCategory::SourceMaterial,
        BackupScope::Full,
    )];
    records[0].expires_at = None;

    assert!(matches!(
        InclusionPlan::build(&request(records)),
        Err(InclusionError::BatchRejected {
            reason: RejectionReason::MissingRetentionMetadata,
            ..
        })
    ));
}

#[test]
fn a_revision_or_hash_mismatch_rejects_the_whole_batch() {
    let mut records = vec![
        record("notes/a.md", SourceCategory::Canonical, BackupScope::Full),
        record(
            "notes/b.md",
            SourceCategory::SourceMaterial,
            BackupScope::Full,
        ),
    ];
    let path = VaultPath::parse("notes/b.md").expect("path");
    records[1].observed_revision = Some(Revision::compute(VaultUuid::new(), &path, b"changed"));

    assert!(matches!(
        InclusionPlan::build(&request(records)),
        Err(InclusionError::BatchRejected {
            reason: RejectionReason::RevisionMismatch,
            ..
        })
    ));
}

#[test]
fn missing_revision_evidence_rejects_the_whole_batch() {
    let mut records = vec![record(
        "notes/b.md",
        SourceCategory::SourceMaterial,
        BackupScope::Full,
    )];
    records[0].declared_revision = None;

    assert!(matches!(
        InclusionPlan::build(&request(records)),
        Err(InclusionError::BatchRejected {
            reason: RejectionReason::MissingRevisionEvidence,
            ..
        })
    ));
}

#[test]
fn canonical_control_audit_full_l0_and_the_deployment_manifests_cannot_be_excluded() {
    for category in [
        SourceCategory::Canonical,
        SourceCategory::ControlDatabase,
        SourceCategory::AuditLedger,
        SourceCategory::FullScopeL0,
        SourceCategory::DeploymentManifest,
        SourceCategory::BackupManifest,
    ] {
        assert!(category.is_mandatory(), "{category:?}");
        let records = vec![record("mandatory.bin", category, BackupScope::SummaryOnly)];
        assert!(
            matches!(
                InclusionPlan::build(&request(records)),
                Err(InclusionError::MandatoryExcluded { .. })
            ),
            "{category:?} 不可被任意排除"
        );
    }
}

#[test]
fn source_material_follows_its_own_scope_expiry_and_crypto_erase_semantics() {
    let mut expired = record("old.md", SourceCategory::SourceMaterial, BackupScope::Full);
    expired.expires_at = Some(InstantMillis::from_millis(500));
    let mut erased = record(
        "erased.md",
        SourceCategory::SourceMaterial,
        BackupScope::Full,
    );
    erased.crypto_erased = true;

    let plan = InclusionPlan::build(&request(vec![
        record("full.md", SourceCategory::SourceMaterial, BackupScope::Full),
        record(
            "summary.md",
            SourceCategory::SourceMaterial,
            BackupScope::SummaryOnly,
        ),
        record("none.md", SourceCategory::SourceMaterial, BackupScope::None),
        expired,
        erased,
    ]))
    .expect("plan");

    let dispositions: Vec<_> = plan.entries.iter().map(|entry| entry.disposition).collect();
    assert_eq!(
        dispositions,
        vec![
            Disposition::FullObject,
            Disposition::RedactedSummary,
            Disposition::NonContentTombstone,
            Disposition::NonContentTombstone,
            Disposition::NonContentTombstone,
        ]
    );
    assert!(plan.requires_filter_staging);
}

#[test]
fn an_excluded_object_requires_a_dedicated_filter_staging_outside_both_roots() {
    let plan = InclusionPlan::build(&request(vec![
        record("full.md", SourceCategory::SourceMaterial, BackupScope::Full),
        record("none.md", SourceCategory::SourceMaterial, BackupScope::None),
    ]))
    .expect("plan");
    assert!(plan.requires_filter_staging);

    let direct = TransferSource {
        root: PathBuf::from("/srv/backup-staging"),
        kind: TransferSourceKind::DirectExport,
        materialization: StagingMaterialization::StreamedCopy,
        expires_at: InstantMillis::from_millis(1_000),
        budget: studiolo_backup::checkpoint::types::StagingBudget::FIXED,
    };
    assert_eq!(
        plan.bind_transfer_source(
            &direct,
            Path::new("/srv/vault"),
            Path::new("/srv/workbench-data"),
            InstantMillis::from_millis(2_000),
        ),
        Err(InclusionError::FilterStagingRequired)
    );

    let mut staging = direct.clone();
    staging.kind = TransferSourceKind::FilteredStaging;
    plan.bind_transfer_source(
        &staging,
        Path::new("/srv/vault"),
        Path::new("/srv/workbench-data"),
        InstantMillis::from_millis(2_000),
    )
    .expect("专用过滤 staging 可以绑定");

    let mut inside = staging.clone();
    inside.root = PathBuf::from("/srv/vault/staging");
    assert_eq!(
        plan.bind_transfer_source(
            &inside,
            Path::new("/srv/vault"),
            Path::new("/srv/workbench-data"),
            InstantMillis::from_millis(2_000),
        ),
        Err(InclusionError::Seal(
            SealError::TransferSourceInsideProtectedRoot
        ))
    );
}

#[test]
fn a_staging_without_the_fixed_budgets_is_refused() {
    let plan = InclusionPlan::build(&request(vec![record(
        "none.md",
        SourceCategory::SourceMaterial,
        BackupScope::None,
    )]))
    .expect("plan");
    let mut staging = TransferSource {
        root: PathBuf::from("/srv/backup-staging"),
        kind: TransferSourceKind::FilteredStaging,
        materialization: StagingMaterialization::StreamedCopy,
        expires_at: InstantMillis::from_millis(1_000),
        budget: studiolo_backup::checkpoint::types::StagingBudget::FIXED,
    };
    staging.budget.reserved_space_bytes = 0;

    assert_eq!(
        plan.bind_transfer_source(
            &staging,
            Path::new("/srv/vault"),
            Path::new("/srv/workbench-data"),
            InstantMillis::from_millis(2_000),
        ),
        Err(InclusionError::Seal(SealError::StagingBudgetMissing))
    );
}

#[test]
fn an_ephemeral_record_can_never_be_planned_into_the_backup() {
    let records = vec![record(
        "run/tmp.bin",
        SourceCategory::Ephemeral,
        BackupScope::Full,
    )];

    assert_eq!(
        InclusionPlan::build(&request(records)),
        Err(InclusionError::EphemeralInsideSourceRoot)
    );
}
