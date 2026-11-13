//! 构建与恢复读取走同一校验：不能在 JSON 往返后绕过固定事实或跨 owner 绑定。
use super::{
    CheckpointManifest, ManifestError, check_watermark, inputs::check_secret_reference,
    require_digest,
};
use studiolo_core::checkpoint::{CheckpointOwner, OwnerAssembly, OwnerWatermark};
pub(super) fn validate(manifest: &CheckpointManifest) -> Result<(), ManifestError> {
    for (owner, assembly) in manifest.roster.entries() {
        let count = manifest
            .watermarks
            .iter()
            .filter(|w| w.owner() == owner)
            .count();
        match (assembly, count) {
            (OwnerAssembly::Present, 1) | (OwnerAssembly::Absent, 0) => {}
            (OwnerAssembly::Present, 0) => return Err(ManifestError::WatermarkMissing { owner }),
            _ => return Err(ManifestError::UnexpectedWatermark { owner }),
        }
    }
    for watermark in &manifest.watermarks {
        check_watermark(watermark)?;
    }
    bindings(manifest)?;
    if manifest.application_packages.is_empty() || manifest.cli_packages.is_empty() {
        return Err(ManifestError::PackageDigestsMissing);
    }
    for package in manifest
        .application_packages
        .iter()
        .chain(&manifest.cli_packages)
    {
        if package.name.trim().is_empty() || package.version.trim().is_empty() {
            return Err(ManifestError::PackageDigestsMissing);
        }
        require_digest(&package.digest, CheckpointOwner::Broker, "package_digest")?;
    }
    if manifest.deployment.is_empty()
        || manifest
            .deployment
            .iter()
            .any(|d| d.unit.trim().is_empty() || d.revision.trim().is_empty())
    {
        return Err(ManifestError::DeploymentManifestMissing);
    }
    for reference in &manifest.secret_references {
        check_secret_reference(reference)?;
    }
    if !manifest.audit_coverage.is_exact() {
        return Err(ManifestError::AuditCoverageNotExact);
    }
    Ok(())
}
fn bindings(manifest: &CheckpointManifest) -> Result<(), ManifestError> {
    let mut primary = None;
    let mut auth = None;
    let mut broker = None;
    let mut control = None;
    for watermark in &manifest.watermarks {
        match watermark {
            OwnerWatermark::Audit {
                last_offset,
                agent_ledger,
                ..
            } => primary = Some((*last_offset, agent_ledger.is_some())),
            OwnerWatermark::Auth {
                audit_anchor_offset,
                ..
            } => auth = Some(*audit_anchor_offset),
            OwnerWatermark::Broker { vault_uuid, .. } => broker = Some(*vault_uuid),
            OwnerWatermark::Control { vault_uuid, .. } => control = Some(*vault_uuid),
            _ => {}
        }
    }
    let agent = manifest.roster.assembly(CheckpointOwner::AgentStore) == OwnerAssembly::Present;
    match primary {
        Some((last, has_agent))
            if has_agent == agent && auth.is_none_or(|offset| offset <= last) => {}
        _ => return Err(ManifestError::AuditLayoutMismatch),
    }
    if control.is_some() && control != broker {
        return Err(ManifestError::VaultBindingMismatch);
    }
    Ok(())
}
