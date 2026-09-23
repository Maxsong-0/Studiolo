//! 副本协议：封闭 JSON 结构、有限资源与时间校验。

use serde::Deserialize;
use studiolo_core::VaultPath;

use super::ReplicaError;

pub(super) const MANIFEST_LIMIT: u64 = 4 * 1024 * 1024;
pub(super) const FILE_LIMIT: u64 = 16 * 1024 * 1024;
pub(super) const NOTE_LIMIT: u64 = 4 * 1024 * 1024;
pub(super) const NOTES_TOTAL_LIMIT: u64 = 16 * 1024 * 1024;
/// 与 scripts/replica_common.py 的 MAX_TOTAL 逐项相等（test-replica-boundaries.py 守卫）；
/// 2026-09-22 T-108 由 128 MiB 上调。附件每次请求只核大小（ADR-023），所以总量上调不增加每请求的哈希开销。
pub(super) const TOTAL_LIMIT: u64 = 512 * 1024 * 1024;
pub(super) const ENTRY_LIMIT: usize = 25_000;

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct State {
    pub schema_version: u8,
    pub status: String,
    pub last_attempt_at_ms: i64,
    #[serde(deserialize_with = "required_option")]
    pub last_error: Option<String>,
    #[serde(deserialize_with = "required_option")]
    pub receipt: Option<Receipt>,
}

/// 接收端已完整验证并原子发布的来源回执。
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Receipt {
    /// 32 位小写十六进制代次标识。
    pub generation: String,
    /// manifest 原始字节的 SHA-256。
    pub source_digest: String,
    /// 源端完成捕获的 Unix 毫秒时间。
    pub source_observed_at_ms: i64,
    /// 接收端确认的 Unix 毫秒时间。
    pub received_at_ms: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Manifest {
    pub schema_version: u8,
    pub allow: Vec<String>,
    pub entries: Vec<ManifestEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ManifestEntry {
    pub path: String,
    pub kind: String,
    pub bytes: u64,
    #[serde(deserialize_with = "required_option")]
    pub sha256: Option<String>,
}

pub(super) fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|ch| ch.is_ascii_digit() || (b'a'..=b'f').contains(&ch))
}

pub(super) fn visible(raw: &str) -> Result<VaultPath, ReplicaError> {
    let path = VaultPath::parse(raw).map_err(|_| ReplicaError::Invalid("noncanonical_path"))?;
    if path.is_protected() || raw.split('/').any(|part| part.starts_with('.')) {
        return Err(ReplicaError::Invalid("hidden_or_protected_path"));
    }
    Ok(path)
}

pub(super) fn allowlist(raw: &[String]) -> Result<Vec<VaultPath>, ReplicaError> {
    if raw.is_empty() || raw.len() > 128 {
        return Err(ReplicaError::Invalid("allowlist_limit"));
    }
    let mut sorted = raw.to_vec();
    sorted.sort();
    let mut paths: Vec<VaultPath> = Vec::new();
    for item in sorted {
        let path = visible(&item)?;
        let lower = item.to_lowercase();
        if paths.iter().any(|existing| {
            let other = existing.as_str().to_lowercase();
            lower == other
                || lower.starts_with(&format!("{other}/"))
                || other.starts_with(&format!("{lower}/"))
        }) {
            return Err(ReplicaError::Invalid("overlapping_allowlist"));
        }
        paths.push(path);
    }
    Ok(paths)
}

impl State {
    pub fn validate(&self, now_ms: i64) -> Result<&Receipt, ReplicaError> {
        if self.schema_version != 1 || !valid_time(self.last_attempt_at_ms, now_ms) {
            return Err(ReplicaError::Invalid("state_schema_or_time"));
        }
        match self.status.as_str() {
            "ready" if self.last_error.is_none() && self.receipt.is_some() => {}
            "failed"
                if matches!(
                    self.last_error.as_deref(),
                    Some("transfer_failed" | "validation_failed")
                ) => {}
            _ => return Err(ReplicaError::Invalid("state_status")),
        }
        let receipt = self
            .receipt
            .as_ref()
            .ok_or(ReplicaError::Invalid("no_verified_replica"))?;
        if !hex(&receipt.generation, 32) || !hex(&receipt.source_digest, 64) {
            return Err(ReplicaError::Invalid("receipt_identifier"));
        }
        if !valid_time(receipt.source_observed_at_ms, now_ms)
            || !valid_time(receipt.received_at_ms, now_ms)
            || receipt.source_observed_at_ms > receipt.received_at_ms
            || receipt.received_at_ms > self.last_attempt_at_ms
        {
            return Err(ReplicaError::Invalid("receipt_time"));
        }
        Ok(receipt)
    }
}

fn valid_time(value: i64, now_ms: i64) -> bool {
    value > 0 && value <= now_ms.saturating_add(30_000)
}

// 供 `deserialize_with` 使用：字段必须出现，同时保留显式给出的 `JSON null`。
fn required_option<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}
