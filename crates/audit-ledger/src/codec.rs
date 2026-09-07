//! v1 规范编码：对象键递归排序，数组有序，UTF-8 紧凑 JSON；磁盘版本不静默改写。

use crate::{AuditError, AuditEvent, AuditRecord, IntegrityHash};
use jiff::Timestamp;
use serde::Serialize;
use serde_json::Value;

pub(crate) const CODEC: &str = "studiolo-audit-json-v1";
pub(crate) const MAX_RECORD_BYTES: usize = 1024 * 1024;

#[cfg(test)]
mod tests;

fn ordered(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(ordered).collect()),
        Value::Object(values) => {
            let sorted: std::collections::BTreeMap<_, _> = values.into_iter().collect();
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(key, value)| (key, ordered(value)))
                    .collect(),
            )
        }
        value => value,
    }
}

pub(crate) fn canonical(value: &impl Serialize) -> Result<String, AuditError> {
    let value = serde_json::to_value(value).map_err(|_| AuditError::Serialization)?;
    serde_json::to_string(&ordered(value)).map_err(|_| AuditError::Serialization)
}

pub(crate) fn build(
    offset: u64,
    recorded_at: Timestamp,
    previous_hash: IntegrityHash,
    event: AuditEvent,
) -> Result<AuditRecord, AuditError> {
    event.payload().validate()?;
    let encoded = canonical(&(offset, recorded_at, previous_hash, &event))?;
    let mut bytes = b"studiolo-audit-record-v1\0".to_vec();
    bytes.extend_from_slice(&previous_hash.bytes());
    bytes.extend_from_slice(encoded.as_bytes());
    Ok(AuditRecord::from_parts(
        offset,
        recorded_at,
        previous_hash,
        IntegrityHash::sha256(&bytes),
        event,
    ))
}

pub(crate) fn decode(encoded: &str) -> Result<AuditRecord, AuditError> {
    serde_json::from_str(encoded).map_err(|_| AuditError::Serialization)
}

pub(crate) fn persistable(record: &AuditRecord) -> Result<String, AuditError> {
    let encoded = canonical(record)?;
    if encoded.len() > MAX_RECORD_BYTES {
        return Err(AuditError::RecordTooLarge);
    }
    // 提交前走真实磁盘解码路径，拒绝默认浮点解析或其它语义无法无损回放的记录。
    let decoded = decode(&encoded)?;
    if &decoded != record || canonical(&decoded)? != encoded {
        return Err(AuditError::Serialization);
    }
    Ok(encoded)
}

pub(crate) fn phase(phase: crate::AuditPhase) -> &'static str {
    match phase {
        crate::AuditPhase::Intent => "intent",
        crate::AuditPhase::Outcome => "outcome",
    }
}
