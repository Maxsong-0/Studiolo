//! 跨 crate 仅传固定类型事件；实际 Audit owner 的核验由可信组合根适配，Broker 不依赖账本。
use crate::{
    BrokerError, ContentAuditEvent, FileOperationAuditEvent, MaintenanceAuditEvent,
    records::WriterRecords,
};
use std::time::Instant;

/// 已保留 ACK 对应的原事件；不能用当前正文重构或改写它。
pub enum BrokerCheckpointAudit<'a> {
    /// 正文操作的固定意图或永久结果。
    Content(&'a ContentAuditEvent),
    /// 类型化文件操作的固定事件。
    FileOperation(&'a FileOperationAuditEvent),
    /// 最后保留的维护槽事件。
    Maintenance(&'a MaintenanceAuditEvent),
}
/// 本 owner 实际核对的回执数和主账本偏移上界；不代表其他 owner 的覆盖。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BrokerCheckpointCoverage {
    checked: u64,
    max_offset: u64,
}
impl BrokerCheckpointCoverage {
    /// 实际调用可信账本核验并成功返回的记录数。
    #[must_use]
    pub fn checked(self) -> u64 {
        self.checked
    }
    /// 所有被核对原回执的最大偏移。
    #[must_use]
    pub fn max_offset(self) -> u64 {
        self.max_offset
    }
}
pub(crate) type Verifier<'a> =
    dyn FnMut(BrokerCheckpointAudit<'_>, u64, &str) -> Result<(), BrokerError> + 'a;
pub(super) fn verify(
    records: &dyn WriterRecords,
    deadline: Instant,
    verify: &mut Verifier<'_>,
) -> Result<BrokerCheckpointCoverage, BrokerError> {
    let mut coverage = BrokerCheckpointCoverage {
        checked: 0,
        max_offset: 0,
    };
    let mut record = |event: BrokerCheckpointAudit<'_>, offset, hash: &str| {
        super::check_deadline(deadline)?;
        verify(event, offset, hash)?;
        coverage.checked += 1;
        coverage.max_offset = coverage.max_offset.max(offset);
        Ok(())
    };
    crate::content_audit::checkpoint::verify(records, deadline, &mut record)?;
    crate::file_operation::checkpoint::verify(records, deadline, &mut record)?;
    crate::maintenance::checkpoint::verify(records, &mut record)?;
    super::check_deadline(deadline)?;
    Ok(coverage)
}
