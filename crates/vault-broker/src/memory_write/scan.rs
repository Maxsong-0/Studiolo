//! 复用 canonical 扫描与水位；不接受目录环境路径。
use super::{MemoryEntryRequest, MemoryPreview, MemoryWriter, plan};
use crate::{BrokerError, FsVaultBroker, MemoryVaultBroker, VaultObject, VaultReader};
fn selected(path: &crate::VaultPath, scope: &crate::VaultPath) -> bool {
    path.collision_key()
        .starts_with(&format!("{}/", scope.collision_key()))
        && path.as_str().ends_with(".md")
}
pub(super) fn filesystem(broker: &FsVaultBroker) -> Result<Vec<VaultObject>, BrokerError> {
    let committed = crate::capture::ready(broker)?;
    let scan = crate::readonly::scan_bound(
        &broker.root,
        crate::ScanLimits::default(),
        broker.vault_uuid,
    )?;
    broker.check_read_watermark(committed)?;
    if !scan.diagnostics.is_empty() {
        return Err(BrokerError::IncompleteCapture);
    }
    Ok(scan
        .notes
        .into_iter()
        .map(|note| VaultObject {
            path: note.path,
            content: note.content,
            revision: note.revision,
        })
        .collect())
}
pub(super) fn memory(broker: &MemoryVaultBroker) -> Result<Vec<VaultObject>, BrokerError> {
    let mut records = Vec::new();
    let mut bytes = 0usize;
    for path in broker
        .objects
        .keys()
        .filter(|path| path.as_str().ends_with(".md"))
    {
        let object = broker.read(path)?;
        bytes = bytes
            .checked_add(object.content.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if records.len() >= 50_000
            || object.content.len() > 4 * 1024 * 1024
            || bytes > 256 * 1024 * 1024
        {
            return Err(BrokerError::WriterLimitExceeded);
        }
        records.push(object);
    }
    Ok(records)
}
impl MemoryWriter for FsVaultBroker {
    fn preview_memory(&self, request: &MemoryEntryRequest) -> Result<MemoryPreview, BrokerError> {
        plan::content(request)?;
        let committed = crate::capture::ready(self)?;
        let records = filesystem(self)?
            .into_iter()
            .filter(|note| selected(&note.path, &request.scope))
            .collect();
        let result = plan::preview(self, request, records)?;
        self.check_read_watermark(committed)?;
        Ok(result)
    }
}
impl MemoryWriter for MemoryVaultBroker {
    fn preview_memory(&self, request: &MemoryEntryRequest) -> Result<MemoryPreview, BrokerError> {
        plan::content(request)?;
        let records = memory(self)?
            .into_iter()
            .filter(|note| selected(&note.path, &request.scope))
            .collect();
        plan::preview(self, request, records)
    }
}
