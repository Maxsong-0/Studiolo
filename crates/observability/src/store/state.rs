//! 死人开关持久文档的纯编解码。没有 IO。
//!
//! 半写、未知版本、未知字段、超限、校验和不符一律**失败关闭**：
//! 返回错误而不是"当作没有状态"，避免把一次撕裂写入洗成"刚刚重置"。

use super::StoreError;
use crate::alert::DeadmanState;
use serde::{Deserialize, Serialize};

/// 持久文档字节上限。状态是定长几个整数，8 KiB 远够用。
pub const STATE_MAX_BYTES: usize = 8 * 1024;

const VERSION: u8 = 1;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    last_receipt: Option<i64>,
    last_observed: i64,
    last_alert: Option<i64>,
    suppressed_since_alert: u32,
    episode: u64,
    dispatch_round: u64,
    checksum: String,
}

fn checksum(
    last_receipt: Option<i64>,
    last_observed: i64,
    last_alert: Option<i64>,
    suppressed_since_alert: u32,
    episode: u64,
    dispatch_round: u64,
) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"studiolo.observability.deadman-state.v1\0");
    digest.update([VERSION]);
    digest.update(last_receipt.unwrap_or(i64::MIN).to_be_bytes());
    digest.update([u8::from(last_receipt.is_some())]);
    digest.update(last_observed.to_be_bytes());
    digest.update(last_alert.unwrap_or(i64::MIN).to_be_bytes());
    digest.update([u8::from(last_alert.is_some())]);
    digest.update(suppressed_since_alert.to_be_bytes());
    digest.update(episode.to_be_bytes());
    digest.update(dispatch_round.to_be_bytes());
    let hex = b"0123456789abcdef";
    digest
        .finalize()
        .iter()
        .flat_map(|byte| {
            [
                char::from(hex[usize::from(byte >> 4)]),
                char::from(hex[usize::from(byte & 15)]),
            ]
        })
        .collect()
}

/// 编码成持久字节；超过预算即失败，不截断。
pub fn encode(state: &DeadmanState) -> Result<Vec<u8>, StoreError> {
    let document = Document {
        version: VERSION,
        last_receipt: state.last_receipt,
        last_observed: state.last_observed,
        last_alert: state.last_alert,
        suppressed_since_alert: state.suppressed_since_alert,
        episode: state.episode,
        dispatch_round: state.dispatch_round,
        checksum: checksum(
            state.last_receipt,
            state.last_observed,
            state.last_alert,
            state.suppressed_since_alert,
            state.episode,
            state.dispatch_round,
        ),
    };
    let bytes = serde_json::to_vec(&document).map_err(|_| StoreError::Corrupt)?;
    if bytes.len() > STATE_MAX_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(bytes)
}

/// 解码持久字节。任何不完整或不认识的内容都失败关闭。
pub fn decode(bytes: &[u8]) -> Result<DeadmanState, StoreError> {
    if bytes.len() > STATE_MAX_BYTES {
        return Err(StoreError::TooLarge);
    }
    let document: Document = serde_json::from_slice(bytes).map_err(|_| StoreError::Corrupt)?;
    if document.version != VERSION {
        return Err(StoreError::UnsupportedSchema);
    }
    let expected = checksum(
        document.last_receipt,
        document.last_observed,
        document.last_alert,
        document.suppressed_since_alert,
        document.episode,
        document.dispatch_round,
    );
    if document.checksum != expected {
        return Err(StoreError::Corrupt);
    }
    Ok(DeadmanState {
        last_receipt: document.last_receipt,
        last_observed: document.last_observed,
        last_alert: document.last_alert,
        suppressed_since_alert: document.suppressed_since_alert,
        episode: document.episode,
        dispatch_round: document.dispatch_round,
    })
}
