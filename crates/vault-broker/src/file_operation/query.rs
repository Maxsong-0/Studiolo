//! 固定主状态与已确认墓碑的查询；不从当前文件猜原操作结果。
use super::*;
use crate::{BrokerError, FsVaultBroker, mutation};

pub(crate) fn status(
    broker: &FsVaultBroker,
    request: &request::Request,
    active: Option<&record::Active>,
    terminal: Option<&record::Terminal>,
) -> Result<FileOperationStatus, BrokerError> {
    let audit_pending = match terminal {
        Some(value) => !audit::confirmed(broker, &audit::outcome(request, value)?)?,
        None => true,
    };
    Ok(FileOperationStatus {
        operation_id: request.frozen.operation_id.clone(),
        phase: active
            .map(|v| v.phase)
            .unwrap_or(FileOperationPhase::FinalizedAwaitAudit),
        audit_pending,
        preview: ticket::preview(
            request,
            terminal.map(|v| v.receipt.outcome).or_else(|| {
                active
                    .filter(|v| v.rejected)
                    .map(|_| FileOperationOutcome::Rejected)
            }),
        ),
        receipt: terminal.map(|v| v.receipt.clone()),
    })
}
impl FsVaultBroker {
    /// 仅返回唯一 typed 活动的元数据；不观察原目标或 stage，便于丢票据后的维护。
    pub fn file_operation_status(&self) -> Result<Option<FileOperationStatus>, BrokerError> {
        let state = mutation::load(self)?;
        state
            .file_operation
            .as_ref()
            .map(|active| {
                status(
                    self,
                    &request::Request::from_active(active)?,
                    Some(active),
                    active.terminal.as_ref(),
                )
            })
            .transpose()
    }
    /// 完整请求/主体/范围绑定的幂等状态；当前原文件随后被更新或移除不会篡改旧回执。
    pub fn file_operation_replay(
        &self,
        request: &FileOperationRequest,
        metadata: &FileOperationMetadata,
    ) -> Result<Option<FileOperationStatus>, BrokerError> {
        let frozen = frozen::Frozen::new(self, request, metadata, store::required(self)?.limits)?;
        let state = mutation::load(self)?;
        if let Some(terminal) = terminal::load(self, &frozen)? {
            let stored = request::load(self, &frozen.operation_id)?
                .ok_or(BrokerError::CorruptWriterState)?;
            return status(
                self,
                &stored,
                state
                    .file_operation
                    .as_ref()
                    .filter(|v| v.frozen.matches(&frozen)),
                Some(&terminal),
            )
            .map(Some);
        }
        if let Some(active) = &state.file_operation
            && active.frozen.key_hash == frozen.key_hash
        {
            if !active.frozen.matches(&frozen) {
                return Err(BrokerError::IdempotencyConflict);
            }
            return status(
                self,
                &request::Request::from_active(active)?,
                Some(active),
                active.terminal.as_ref(),
            )
            .map(Some);
        }
        super::legacy_key(self, &frozen.key_hash)?;
        if request::load(self, &frozen.operation_id)?.is_some() {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(None)
    }
    /// 已确认删除的墓碑有界分页；此版本没有 GC 或重定向恢复路径的入口。
    pub fn list_tombstones(
        &self,
        limit: usize,
        cursor: Option<&str>,
    ) -> Result<TombstonePage, BrokerError> {
        if !(1..=100).contains(&limit) {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let state = mutation::load(self)?;
        if self.write_status()?.read_blocked {
            return Err(BrokerError::MutationUnresolved);
        }
        let names = disk::names(self, disk::Folder::Tombstones)?;
        let generation = digest(&(self.writer.binding.pair_id, &names, state.committed_through))?;
        let start = match cursor {
            None => 0,
            Some(value) => parse_cursor(value, &generation)?,
        };
        let mut entries = Vec::new();
        for name in &names {
            let value = tombstone::load(self, name.trim_end_matches(".json"))?;
            let request = request::load(self, &value.terminal.frozen.operation_id)?
                .ok_or(BrokerError::CorruptWriterState)?;
            if audit::confirmed(self, &audit::outcome(&request, &value.terminal)?)? {
                entries.push(value.entry()?);
            }
        }
        entries.sort_by(|a, b| {
            b.deleted_at_ms
                .cmp(&a.deleted_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        if start > entries.len() {
            return Err(BrokerError::InvalidHistoryCursor);
        }
        let end = start.saturating_add(limit).min(entries.len());
        let next_cursor = (end < entries.len()).then(|| format!("d1:{generation}:{end}"));
        if digest(&mutation::load(self)?)? != digest(&state)?
            || disk::names(self, disk::Folder::Tombstones)? != names
        {
            return Err(BrokerError::HistoryChanged);
        }
        Ok(TombstonePage {
            entries: entries[start..end].to_vec(),
            next_cursor,
        })
    }
}
fn parse_cursor(value: &str, generation: &str) -> Result<usize, BrokerError> {
    if value.len() > 100 {
        return Err(BrokerError::InvalidHistoryCursor);
    }
    let fields: Vec<_> = value.split(':').collect();
    if fields.len() != 3
        || fields[0] != "d1"
        || !crate::writer::disk::hex(fields[1])
        || fields[2].is_empty()
        || !fields[2].bytes().all(|v| v.is_ascii_digit())
    {
        return Err(BrokerError::InvalidHistoryCursor);
    }
    let offset = fields[2]
        .parse()
        .map_err(|_| BrokerError::InvalidHistoryCursor)?;
    if fields[1] != generation {
        return Err(BrokerError::HistoryChanged);
    }
    Ok(offset)
}
