//! 固定准入事件映射；只带来源摘要和原链截止点，不接受客户端自由载荷。
use super::*;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use studiolo_audit_ledger::{AuditEventId, IntegrityHash, SanitizedPayload};
use studiolo_core::checkpoint::{CheckpointId, OwnerWatermark};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AdmissionRecord {
    schema: u32,
    pub key: JobKey,
    pub target: BackupTargetKind,
    pub source: SealedReference,
    pub content_cutoff: InstantMillis,
    pub expires_at: InstantMillis,
    pub audit_offset: u64,
    pub audit_hash: String,
}
impl AdmissionRecord {
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
    pub fn event_id(id: &CheckpointId) -> Result<AuditEventId, AuditError> {
        let hash = IntegrityHash::sha256(id.as_str().as_bytes()).to_hex();
        AuditEventId::parse(format!("backup-admission-{hash}"))
    }
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
    pub fn from_receipt(receipt: &AuditRecord) -> Result<Self, AuditError> {
        receipt.verify_integrity()?;
        let record: Self = serde_json::from_value(receipt.event().payload().as_value().clone())
            .map_err(|_| AuditError::CorruptStore)?;
        verify_receipt(receipt, &record.event()?, record.audit_offset)?;
        Ok(record)
    }
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
