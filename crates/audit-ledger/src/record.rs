use crate::AuditEvent;
use jiff::Timestamp;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// A SHA-256 digest used by the append-only integrity chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IntegrityHash([u8; 32]);

impl IntegrityHash {
    pub(crate) fn bytes(self) -> [u8; 32] {
        self.0
    }

    /// Computes a SHA-256 digest.
    #[must_use]
    pub fn sha256(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// Returns the fixed genesis digest for a new ledger.
    #[must_use]
    pub fn genesis() -> Self {
        Self::sha256(b"studiolo-audit-genesis-v1")
    }

    /// 解析恰好 64 个小写十六进制字符；长度、大小写或字符不符一律返回 `None`。
    ///
    /// 仅供持久 Adapter 还原自己写下的摘要；宽松形式不接受，避免同一摘要有多种文本表示。
    pub(crate) fn from_hex(text: &str) -> Option<Self> {
        if text.len() != 64 {
            return None;
        }
        let mut bytes = [0_u8; 32];
        let source = text.as_bytes();
        for (index, byte) in bytes.iter_mut().enumerate() {
            let high = digit(source[index * 2])?;
            let low = digit(source[index * 2 + 1])?;
            *byte = high * 16 + low;
        }
        Some(Self(bytes))
    }

    /// Returns the lower-case hexadecimal representation.
    #[must_use]
    pub fn to_hex(self) -> String {
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write as _;
            write!(&mut output, "{byte:02x}").expect("writing to String cannot fail");
        }
        output
    }
}

/// 只接受 `0-9` 与小写 `a-f`；大写或其它字符返回 `None`。
fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

impl fmt::Display for IntegrityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_hex())
    }
}

/// Durable result returned by an append operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    offset: u64,
    recorded_at: Timestamp,
    previous_hash: IntegrityHash,
    record_hash: IntegrityHash,
    event: AuditEvent,
}

impl AuditRecord {
    /// 检查单条回执的偏移、创世前驱和内容哈希是否自洽。
    ///
    /// 复用本模块固定编码；失败不返回部分可信结论。本方法不访问权威存储，
    /// 不证明记录来自某个账本，也不验证相邻记录或抵御整体重写。
    pub fn verify_integrity(&self) -> Result<(), crate::AuditError> {
        if self.offset == 0 || (self.offset == 1 && self.previous_hash != IntegrityHash::genesis())
        {
            return Err(crate::AuditError::CorruptStore);
        }
        let expected = crate::codec::build(
            self.offset,
            self.recorded_at,
            self.previous_hash,
            self.event.clone(),
        )?;
        if expected.record_hash != self.record_hash {
            return Err(crate::AuditError::CorruptStore);
        }
        Ok(())
    }

    /// Reconstructs a record from authoritative adapter-owned storage parts.
    ///
    /// The adapter is responsible for validating the offset and integrity hash
    /// before exposing the record through [`AuditLedger`](crate::AuditLedger).
    #[must_use]
    pub fn from_parts(
        offset: u64,
        recorded_at: Timestamp,
        previous_hash: IntegrityHash,
        record_hash: IntegrityHash,
        event: AuditEvent,
    ) -> Self {
        Self {
            offset,
            recorded_at,
            previous_hash,
            record_hash,
            event,
        }
    }

    /// Returns the one-based append offset.
    #[must_use]
    pub fn offset(&self) -> u64 {
        self.offset
    }

    /// Returns the server-side recording timestamp.
    #[must_use]
    pub fn recorded_at(&self) -> Timestamp {
        self.recorded_at
    }

    /// Returns the preceding record hash, or the genesis hash for offset one.
    #[must_use]
    pub fn previous_hash(&self) -> IntegrityHash {
        self.previous_hash
    }

    /// Returns this record's SHA-256 chain digest.
    #[must_use]
    pub fn record_hash(&self) -> IntegrityHash {
        self.record_hash
    }

    /// Returns the appended semantic event.
    #[must_use]
    pub fn event(&self) -> &AuditEvent {
        &self.event
    }
}
