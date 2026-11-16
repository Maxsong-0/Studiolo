//! 只累计 FrozenAudit 真正核对过的回执，调用者不能凭 offset 创建成功证明。
use crate::AuditError;

/// 一条已核对的回执；只能由 FrozenAudit 构造。
pub struct VerifiedReceipt {
    pub(super) offset: u64,
}

/// owner 对自己所有持久 ACK 的实际逐条核对统计。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReceiptCoverage {
    checked: u64,
    maximum: u64,
}
impl ReceiptCoverage {
    /// 累计一条已核对的原事件；重复引用仍计一次实际核对，不推测全表覆盖。
    pub fn include(&mut self, receipt: VerifiedReceipt) -> Result<(), AuditError> {
        self.checked = self
            .checked
            .checked_add(1)
            .ok_or(AuditError::CorruptStore)?;
        self.maximum = self.maximum.max(receipt.offset);
        Ok(())
    }
    /// 合并不同 owner 的实际核对次数；不同账本的最大偏移不能据此充当共同水位。
    pub fn combine(&mut self, other: Self) -> Result<(), AuditError> {
        self.checked = self
            .checked
            .checked_add(other.checked)
            .ok_or(AuditError::CorruptStore)?;
        self.maximum = self.maximum.max(other.maximum);
        Ok(())
    }
    /// 本 owner 实际核对过的引用数。
    pub fn checked(&self) -> u64 {
        self.checked
    }
    /// 同一账本中的最高原回执偏移。
    pub fn maximum_offset(&self) -> u64 {
        self.maximum
    }
}
