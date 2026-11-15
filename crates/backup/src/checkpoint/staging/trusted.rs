//! 把 Broker 给出的来源事实组装成封存目录。本模块不读文件，也不补默认完整范围。
use super::{Result, SourceCatalog, StagingError};
use crate::checkpoint::inclusion::{SourceCategory, SourceRecord};
use crate::checkpoint::staging::ObjectRegistration;
use studiolo_core::{
    BackupScope, DataLabels, TrustedExportClass, TrustedExportFact, VaultUuid,
    checkpoint::InstantMillis,
};

/// 组装最终来源目录。尝试正文没有原批准时整批拒绝。
pub fn trusted_source_catalog(
    vault: VaultUuid,
    facts: impl IntoIterator<Item = TrustedExportFact>,
) -> Result<SourceCatalog> {
    let mut records = Vec::new();
    for fact in facts {
        records.push(registration(vault, fact)?);
    }
    SourceCatalog::new(records)
}

fn registration(vault: VaultUuid, fact: TrustedExportFact) -> Result<ObjectRegistration> {
    let category = match fact.class {
        TrustedExportClass::SourceMaterial => SourceCategory::SourceMaterial,
        TrustedExportClass::FullScopeL0 => SourceCategory::FullScopeL0,
        TrustedExportClass::ControlDatabase => SourceCategory::ControlDatabase,
        TrustedExportClass::AuditLedger => SourceCategory::AuditLedger,
        TrustedExportClass::Canonical => SourceCategory::Canonical,
    };
    let (source_policy, labels, expires_at) = if category == SourceCategory::SourceMaterial {
        let provenance = fact.provenance.as_ref().ok_or(StagingError::Catalog)?;
        (
            Some(provenance.source_policy().clone()),
            provenance.labels().clone(),
            Some(InstantMillis::from_millis(
                provenance
                    .retain_until_unix_seconds()
                    .checked_mul(1000)
                    .ok_or(StagingError::Catalog)?,
            )),
        )
    } else if let Some(provenance) = fact.provenance.as_ref() {
        (
            Some(provenance.source_policy().clone()),
            provenance.labels().clone(),
            None,
        )
    } else {
        (None, owner_labels()?, None)
    };
    if category == SourceCategory::FullScopeL0 && labels.backup_scope() != BackupScope::Full {
        return Err(StagingError::Catalog);
    }
    Ok(ObjectRegistration {
        export_path: fact.export_path.clone(),
        vault_uuid: vault,
        source: SourceRecord {
            path: fact.export_path,
            category,
            source_policy,
            labels: Some(labels),
            expires_at,
            crypto_erased: false,
            declared_revision: Some(fact.revision),
            observed_revision: None,
        },
        redacted: None,
    })
}

fn owner_labels() -> Result<DataLabels> {
    DataLabels::new(
        studiolo_core::Classification::Restricted,
        BackupScope::Full,
        studiolo_core::DesktopSync::Deny,
        studiolo_core::ModelEgress::Deny,
    )
    .map_err(|_| StagingError::Catalog)
}
