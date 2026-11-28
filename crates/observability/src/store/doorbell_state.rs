//! 出站门铃持久文档的纯编解码。没有 IO。
//!
//! 和死人开关状态一样失败关闭：半写、未知版本、未知字段、超限、校验和不符都返回错误，
//! 不当成「还没启用」，否则一次撕裂写入会让门铃从账本最新位置重新开始、悄悄漏掉事件。

use super::{STATE_MAX_BYTES, StoreError};
use crate::doorbell::DoorbellState;
use serde::{Deserialize, Serialize};

const VERSION: u8 = 1;
/// 收件地址指纹的固定长度（十六进制字符数）。
const TAG_LENGTH: usize = 16;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    version: u8,
    cursor: u64,
    attempts: u32,
    retry_at_ms: i64,
    gone: bool,
    endpoint_tag: String,
    checksum: String,
}

fn checksum(state: &DoorbellState) -> String {
    use sha2::{Digest, Sha256};
    let mut digest = Sha256::new();
    digest.update(b"studiolo.observability.doorbell-state.v1\0");
    digest.update([VERSION]);
    digest.update(state.cursor.to_be_bytes());
    digest.update(state.attempts.to_be_bytes());
    digest.update(state.retry_at_ms.to_be_bytes());
    digest.update([u8::from(state.gone)]);
    digest.update(state.endpoint_tag.as_bytes());
    studiolo_core::to_lower_hex(&digest.finalize())
}

fn valid_tag(tag: &str) -> bool {
    tag.len() == TAG_LENGTH && studiolo_core::is_lower_hex(tag)
}

/// 编码成持久字节；指纹形状不对或超过预算即失败，不截断。
pub fn encode(state: &DoorbellState) -> Result<Vec<u8>, StoreError> {
    if !valid_tag(&state.endpoint_tag) {
        return Err(StoreError::Corrupt);
    }
    let document = Document {
        version: VERSION,
        cursor: state.cursor,
        attempts: state.attempts,
        retry_at_ms: state.retry_at_ms,
        gone: state.gone,
        endpoint_tag: state.endpoint_tag.clone(),
        checksum: checksum(state),
    };
    let bytes = serde_json::to_vec(&document).map_err(|_| StoreError::Corrupt)?;
    if bytes.len() > STATE_MAX_BYTES {
        return Err(StoreError::TooLarge);
    }
    Ok(bytes)
}

/// 解码持久字节。任何不完整或不认识的内容都失败关闭。
pub fn decode(bytes: &[u8]) -> Result<DoorbellState, StoreError> {
    if bytes.len() > STATE_MAX_BYTES {
        return Err(StoreError::TooLarge);
    }
    let document: Document = serde_json::from_slice(bytes).map_err(|_| StoreError::Corrupt)?;
    if document.version != VERSION {
        return Err(StoreError::UnsupportedSchema);
    }
    let state = DoorbellState {
        cursor: document.cursor,
        attempts: document.attempts,
        retry_at_ms: document.retry_at_ms,
        gone: document.gone,
        endpoint_tag: document.endpoint_tag,
    };
    if !valid_tag(&state.endpoint_tag) || document.checksum != checksum(&state) {
        return Err(StoreError::Corrupt);
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DoorbellState {
        DoorbellState {
            cursor: 41,
            attempts: 2,
            retry_at_ms: 1_700_000_000_000,
            gone: false,
            endpoint_tag: "0123456789abcdef".into(),
        }
    }

    #[test]
    fn a_saved_state_reads_back_exactly() {
        let bytes = encode(&sample()).unwrap();
        assert_eq!(decode(&bytes).unwrap(), sample());
    }

    #[test]
    fn torn_edited_or_unknown_documents_fail_closed() {
        let bytes = encode(&sample()).unwrap();
        assert_eq!(decode(&bytes[..bytes.len() - 1]), Err(StoreError::Corrupt));
        // 改了位置但没改校验和：不当成合法状态。
        let edited = String::from_utf8(bytes.clone())
            .unwrap()
            .replace("\"cursor\":41", "\"cursor\":40");
        assert_eq!(decode(edited.as_bytes()), Err(StoreError::Corrupt));
        let unknown = String::from_utf8(bytes.clone())
            .unwrap()
            .replace("{\"version\":1", "{\"extra\":0,\"version\":1");
        assert_eq!(decode(unknown.as_bytes()), Err(StoreError::Corrupt));
        let future = String::from_utf8(bytes)
            .unwrap()
            .replace("\"version\":1", "\"version\":2");
        assert_eq!(
            decode(future.as_bytes()),
            Err(StoreError::UnsupportedSchema)
        );
        assert_eq!(
            decode(&vec![b' '; STATE_MAX_BYTES + 1]),
            Err(StoreError::TooLarge)
        );
    }

    #[test]
    fn a_malformed_endpoint_tag_is_never_written_or_accepted() {
        for tag in [
            "",
            "0123456789abcde",
            "0123456789ABCDEF",
            "0123456789abcdeg",
        ] {
            let state = DoorbellState {
                endpoint_tag: tag.into(),
                ..sample()
            };
            assert_eq!(encode(&state), Err(StoreError::Corrupt));
        }
    }
}
