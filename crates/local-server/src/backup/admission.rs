//! 只由可信协调器批准实际封存；借原权威账本持久化，不引入第二份准入状态文件。
use std::sync::Arc;
use studiolo_audit_ledger::{
    AuditError, AuditEvent, AuditFilter, AuditLedger, AuditPhase, AuditRecord, SqliteAuditLedger,
};
use studiolo_backup::worker::audit::AdmissionFact;
use studiolo_backup::{
    checkpoint::staging::{SealedCheckpoint, SealedReference},
    worker::{job::backup_intent, recovery_point::BackupTargetKind, state::JobKey},
};
use studiolo_core::checkpoint::InstantMillis;

/// 已持久且绑定原账本的准入能力；请求消息不能反序列化或直接构造它。
pub struct BackupAdmission {
    pub(super) ledger: Arc<SqliteAuditLedger>,
    record: AdmissionFact,
}
impl BackupAdmission {
    /// 可信 L2/调度协调器在释放冻结窗口后调用；原 Intent 必须已落在本次封存链内。
    /// 只记录原封存摘要、期限和审计截止点，不复制正文或扩大其保留范围。
    pub fn approve(
        ledger: Arc<SqliteAuditLedger>,
        key: JobKey,
        target: BackupTargetKind,
        sealed: &SealedCheckpoint,
    ) -> Result<Self, AuditError> {
        let record = AdmissionFact::from_sealed(key, target, sealed)?;
        let _operation = ledger.enter_operation()?;
        record.check_live()?;
        verify_source(&ledger, &record)?;
        let event = record.event()?;
        let receipt = ledger.append(event.clone())?;
        verify_receipt(&receipt, &event, record.audit_offset())?;
        record.check_live()?;
        Ok(Self {
            ledger: ledger.clone(),
            record,
        })
    }
    /// 重启只读恢复；缺少原准入或原 Intent 失败关闭，绝不补造批准。
    /// 过期后仍可核对远端和补审计，但不得重新交付正文，后者须调用 check_live。
    pub fn resume(
        ledger: Arc<SqliteAuditLedger>,
        key: &JobKey,
        target: BackupTargetKind,
    ) -> Result<Self, AuditError> {
        let _operation = ledger.enter_operation()?;
        let receipt = query_one(&ledger, &AdmissionFact::event_id(&key.checkpoint_id)?)?
            .ok_or(AuditError::Unavailable)?;
        let record = AdmissionFact::from_receipt(&receipt)?;
        if record.key() != key || record.target() != target {
            return Err(AuditError::CorruptStore);
        }
        let admission = Self {
            ledger: ledger.clone(),
            record,
        };
        admission.verify()?;
        Ok(admission)
    }
    /// 原批准作业；不能用新的 sequence 替换同一 checkpoint。
    pub fn key(&self) -> &JobKey {
        self.record.key()
    }
    /// 已批准的固定目标类别。
    pub fn target(&self) -> BackupTargetKind {
        self.record.target()
    }
    /// 原封存摘要；worker 必须按此引用重开，不能扫描并选择最新目录。
    pub fn source(&self) -> &SealedReference {
        self.record.source()
    }
    /// 原正文销毁期限；续办不能延长。
    pub fn expires_at(&self) -> InstantMillis {
        self.record.expires_at()
    }
    /// 仅新交付正文要求仍在原期限内；纯远端核对和审计允许在过期后继续。
    pub fn check_live(&self) -> Result<(), AuditError> {
        self.record.check_live()
    }
    pub(super) fn receipt(&self) -> Result<AuditRecord, AuditError> {
        let event = self.record.event()?;
        let receipt = query_one(&self.ledger, event.event_id())?.ok_or(AuditError::Unavailable)?;
        verify_receipt(&receipt, &event, self.record.audit_offset())?;
        Ok(receipt)
    }
    pub(super) fn verify(&self) -> Result<(), AuditError> {
        verify_source(&self.ledger, &self.record)?;
        let expected = self.record.event()?;
        let receipt =
            query_one(&self.ledger, expected.event_id())?.ok_or(AuditError::Unavailable)?;
        verify_receipt(&receipt, &expected, self.record.audit_offset())
    }
}
fn verify_source(ledger: &SqliteAuditLedger, record: &AdmissionFact) -> Result<(), AuditError> {
    record.validate()?;
    let expected =
        backup_intent(record.key(), record.target()).map_err(|_| AuditError::Serialization)?;
    let intent = query_one(ledger, expected.event_id())?.ok_or(AuditError::Unavailable)?;
    if intent.event() != &expected || intent.offset() > record.audit_offset() {
        return Err(AuditError::CorruptStore);
    }
    let mut tails = ledger.query(
        &AuditFilter::default()
            .after_offset(record.audit_offset() - 1)
            .with_limit(1),
    )?;
    let tail = tails.pop().ok_or(AuditError::CorruptStore)?;
    tail.verify_integrity()?;
    if tail.offset() != record.audit_offset() || tail.record_hash().to_hex() != record.audit_hash()
    {
        return Err(AuditError::CorruptStore);
    }
    Ok(())
}
fn query_one(
    ledger: &SqliteAuditLedger,
    id: &studiolo_audit_ledger::AuditEventId,
) -> Result<Option<AuditRecord>, AuditError> {
    let mut records = ledger.query(
        &AuditFilter::default()
            .with_event_id(id.clone())
            .with_phase(AuditPhase::Intent)
            .with_limit(2),
    )?;
    if records.len() > 1 {
        return Err(AuditError::CorruptStore);
    }
    let receipt = records.pop();
    if let Some(receipt) = &receipt {
        receipt.verify_integrity()?;
    }
    Ok(receipt)
}
fn verify_receipt(
    receipt: &AuditRecord,
    expected: &AuditEvent,
    cutoff: u64,
) -> Result<(), AuditError> {
    receipt.verify_integrity()?;
    if receipt.event() != expected || receipt.offset() <= cutoff {
        return Err(AuditError::CorruptStore);
    }
    Ok(())
}
