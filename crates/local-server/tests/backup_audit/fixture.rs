//! 重开/损坏契约直接构造既有磁盘事件；真实 approve 正向另由实际 owner 封存测试覆盖。
use super::*;
use studiolo_audit_ledger::{AuditEventId, IntegrityHash, SanitizedPayload};
pub fn grant(
    ledger: &SqliteAuditLedger,
    mutate: impl FnOnce(&mut serde_json::Value),
) -> AuditEvent {
    let expected = backup_intent(&key(), TARGET).unwrap();
    let original = ledger.append(expected).unwrap();
    let mut body = json!({
        "schema":1,"key":key(),"target":TARGET,
        "source":{"checkpoint_id":key().checkpoint_id,"seal_sha256":"ab".repeat(32)},
        "content_cutoff":0,"expires_at":i64::MAX,
        "audit_offset":original.offset(),"audit_hash":original.record_hash().to_hex(),
    });
    mutate(&mut body);
    let hash = IntegrityHash::sha256(key().checkpoint_id.as_str().as_bytes()).to_hex();
    AuditEvent::new(
        AuditEventId::parse(format!("backup-admission-{hash}")).unwrap(),
        AuditPhase::Intent,
        "backup.snapshot",
        SanitizedPayload::try_from(body).unwrap(),
    )
}
pub fn existing(ledger: &Arc<SqliteAuditLedger>) -> BackupAdmission {
    ledger.append(grant(ledger, |_| {})).unwrap();
    BackupAdmission::resume(ledger.clone(), &key(), TARGET).unwrap()
}
