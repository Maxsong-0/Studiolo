//! v5 增加互斥文件操作，v6 显式启用 Rename；旧字段字节和指纹不迁移。
//! v9 有打包的旧记录（`crate::archive`）：只在第一次打包时写，没打过包的状态不变，旧程序照样能开。
use super::Operation;
use crate::{BrokerError, VaultUuid};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct State {
    pub(crate) format_version: u32,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) maintenance_enabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) file_operations_enabled: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) rename_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) file_operation: Option<crate::file_operation::record::Active>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) batch: Option<crate::batch::record::Active>,
    /// 打包的段头；存它就是一步打包的提交点。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) archive: Option<crate::archive::Head>,
    pub(crate) pair_id: VaultUuid,
    pub(crate) committed_through: u64,
    pub(crate) next_sequence: u64,
    pub(crate) legacy_unresolved: bool,
    pub(crate) operation: Option<Operation>,
}

impl State {
    pub(crate) fn initial(pair_id: VaultUuid, legacy_unresolved: bool) -> Self {
        Self {
            format_version: 1,
            maintenance_enabled: false,
            file_operations_enabled: false,
            rename_enabled: false,
            file_operation: None,
            batch: None,
            archive: None,
            pair_id,
            committed_through: 0,
            next_sequence: 1,
            legacy_unresolved,
            operation: None,
        }
    }
    pub(crate) fn validate(&self, pair: VaultUuid) -> Result<(), BrokerError> {
        if !matches!(self.format_version, 1..=9)
            || (self.format_version == 9) != self.archive.is_some()
            || (self.format_version < 4 && (self.format_version == 3) != self.maintenance_enabled)
            || (self.format_version < 7
                && (self.format_version >= 5) != self.file_operations_enabled)
            || (self.format_version < 7 && (self.format_version == 6) != self.rename_enabled)
            || (self.rename_enabled && !self.file_operations_enabled)
            || (!self.file_operations_enabled && self.file_operation.is_some())
            || (self.file_operation.is_some()
                && (self.operation.is_some() || self.legacy_unresolved))
            || self.pair_id != pair
            || self.next_sequence <= self.committed_through
        {
            return Err(BrokerError::CorruptWriterState);
        }
        if let Some(head) = &self.archive {
            head.validate()?;
        }
        if let Some(batch) = &self.batch {
            batch.validate()?;
            if self.format_version < 7
                || self.operation.is_some()
                || self.file_operation.is_some()
                || self.legacy_unresolved
                || batch.sequence <= self.committed_through
                || batch.sequence.checked_add(1) != Some(self.next_sequence)
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        if let Some(op) = &self.operation {
            op.validate(self.format_version >= 4 && op.recording.is_some())?;
            if self.legacy_unresolved
                || (self.format_version < 3
                    && ((self.format_version == 2) != op.recording.is_some()))
                || op.sequence <= self.committed_through
                || op.sequence.checked_add(1) != Some(self.next_sequence)
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        if let Some(op) = &self.file_operation {
            op.validate(pair)?;
            if op.frozen.version == 2 && !self.rename_enabled {
                return Err(BrokerError::CorruptWriterState);
            }
            let committed = op
                .terminal
                .as_ref()
                .is_some_and(|v| v.receipt.outcome == crate::FileOperationOutcome::Committed);
            if op.sequence.checked_add(1) != Some(self.next_sequence)
                || (committed && op.sequence != self.committed_through)
                || (!committed && op.sequence <= self.committed_through)
            {
                return Err(BrokerError::CorruptWriterState);
            }
        }
        Ok(())
    }
}
