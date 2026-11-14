//! 固定准入事件映射；只带来源摘要和原链截止点，不接受客户端自由载荷。
use crate::{
    checkpoint::staging::{SealedCheckpoint, SealedReference},
    worker::{recovery_point::BackupTargetKind, state::JobKey},
};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditPhase, AuditRecord};
use studiolo_audit_ledger::{AuditEventId, IntegrityHash, SanitizedPayload};
use studiolo_core::checkpoint::{CheckpointId, InstantMillis, OwnerWatermark};

/// 封存准入的固定事实；通过完整回执验证只证明消息自洽，授权仍来自可信生产者。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdmissionFact {
    schema: u32,
    key: JobKey,
    target: BackupTargetKind,
    source: SealedReference,
    content_cutoff: InstantMillis,
    expires_at: InstantMillis,
    audit_offset: u64,
    audit_hash: String,
}
impl AdmissionFact {
    /// 原作业键。
    pub fn key(&self) -> &JobKey {
        &self.key
    }
    /// 原目标类别。
    pub fn target(&self) -> BackupTargetKind {
        self.target
    }
    /// 原封存摘要。
    pub fn source(&self) -> &SealedReference {
        &self.source
    }
    /// 原正文销毁期限。
    pub fn expires_at(&self) -> InstantMillis {
        self.expires_at
    }
    /// 原人侧审计链截止 offset。
    pub fn audit_offset(&self) -> u64 {
        self.audit_offset
    }
    /// 原人侧审计链截止哈希。
    pub fn audit_hash(&self) -> &str {
        &self.audit_hash
    }
    /// 只从已验证封存取事实；本方法不签发权限或持久批准。
    pub fn from_sealed(
        key: JobKey,
        target: BackupTargetKind,
        sealed: &SealedCheckpoint,
    ) -> Result<Self, AuditError> {
        let (audit_offset, audit_hash) = sealed
            .manifest()
            .watermarks
            .iter()
            .find_map(|w| {
                if let OwnerWatermark::Audit {
                    last_offset,
                    last_hash,
                    ..
                } = w
                {
                    Some((*last_offset, last_hash.clone()))
                } else {
                    None
                }
            })
            .ok_or(AuditError::CorruptStore)?;
        let record = Self {
            schema: 1,
            key,
            target,
            source: sealed.reference().clone(),
            content_cutoff: sealed.ticket().content_cutoff,
            expires_at: sealed.ticket().transfer_source.expires_at,
            audit_offset,
            audit_hash,
        };
        record.validate()?;
        Ok(record)
    }
    /// 固定 checkpoint 唯一事件 ID。
    /// 重新生成完整固定事件。
    pub fn event_id(id: &CheckpointId) -> Result<AuditEventId, AuditError> {
        let hash = IntegrityHash::sha256(id.as_str().as_bytes()).to_hex();
        AuditEventId::parse(format!("backup-admission-{hash}"))
    }
    /// 重新生成完整固定事件。
    pub fn event(&self) -> Result<AuditEvent, AuditError> {
        self.validate()?;
        Ok(AuditEvent::new(
            Self::event_id(&self.key.checkpoint_id)?,
            AuditPhase::Intent,
            "backup.snapshot",
            SanitizedPayload::try_from(
                serde_json::to_value(self).map_err(|_| AuditError::Serialization)?,
            )?,
        ))
    }
    /// 验证原回执形状、完整性和截止关系；不能替代真实 peer 鉴权。
    pub fn from_receipt(receipt: &AuditRecord) -> Result<Self, AuditError> {
        receipt.verify_integrity()?;
        let record: Self = serde_json::from_value(receipt.event().payload().as_value().clone())
            .map_err(|_| AuditError::CorruptStore)?;
        if receipt.event() != &record.event()? || receipt.offset() <= record.audit_offset {
            return Err(AuditError::CorruptStore);
        }
        Ok(record)
    }
    /// 核对全部字段之间的固定关系。
    pub fn validate(&self) -> Result<(), AuditError> {
        if self.schema != 1
            || self.key.checkpoint_id != self.source.checkpoint_id
            || self.content_cutoff.millis() < 0
            || self.expires_at <= self.content_cutoff
            || self.audit_offset == 0
            || self.audit_offset > i64::MAX as u64
            || !digest(&self.source.seal_sha256)
            || !digest(&self.audit_hash)
        {
            return Err(AuditError::CorruptStore);
        }
        Ok(())
    }
    /// 新交付正文必须仍在原时间窗内；过期可继续非正文核对。
    pub fn check_live(&self) -> Result<(), AuditError> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|v| i64::try_from(v.as_millis()).ok())
            .map(InstantMillis::from_millis)
            .ok_or(AuditError::Unavailable)?;
        if now < self.content_cutoff || now >= self.expires_at {
            return Err(AuditError::Unavailable);
        }
        Ok(())
    }
}
fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
