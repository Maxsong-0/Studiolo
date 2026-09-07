//! 种子与验证码的零化封装；种子引用只作标识，绝不解释为文件路径。

use crate::session::SessionError;
use sha2::{Digest, Sha256};
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// 可信装配提供的 160 位 TOTP 种子；不序列化、克隆或输出种子及引用。
pub struct TotpSeed {
    reference: String,
    secret: Zeroizing<[u8; 20]>,
}

impl TotpSeed {
    /// 解码恰好 40 位小写 ASCII hex；不读取环境、不修剪或隐式规范化输入。
    ///
    /// 编码无效返回 `Rejected`；引用仍由 `new` 检查并返回 `Configuration`。
    /// 自有的 20 字节解码缓冲区会零化；借用的输入仍须由调用者保护和清除。
    pub fn from_hex(reference: String, encoded: &[u8]) -> Result<Self, SessionError> {
        if encoded.len() != 40
            || !encoded
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(SessionError::Rejected);
        }
        let mut bytes = Zeroizing::new([0_u8; 20]);
        let nibble = |byte: u8| {
            if byte.is_ascii_digit() {
                byte - b'0'
            } else {
                byte - b'a' + 10
            }
        };
        for (byte, pair) in bytes.iter_mut().zip(encoded.chunks_exact(2)) {
            *byte = nibble(pair[0]) * 16 + nibble(pair[1]);
        }
        Self::new(reference, *bytes)
    }

    /// 绑定 1～64 位 ASCII 字母、数字、点、下划线或短横线组成的非路径引用。
    ///
    /// 只保护本实例拥有的缓冲区，不替调用方清除其自行保留的种子副本。
    pub fn new(reference: String, bytes: [u8; 20]) -> Result<Self, SessionError> {
        let secret = Zeroizing::new(bytes);
        if reference.is_empty()
            || reference.len() > 64
            || !reference
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Err(SessionError::Configuration);
        }
        Ok(Self { reference, secret })
    }

    pub(in crate::session) fn reference(&self) -> &str {
        &self.reference
    }

    pub(in crate::session) fn bytes(&self) -> &[u8; 20] {
        &self.secret
    }

    pub(in crate::session) fn fingerprint(&self) -> String {
        self.fingerprint_for(&self.reference)
    }

    pub(in crate::session) fn matches_binding(&self, reference: &str, digest: &str) -> bool {
        // 按旧引用绑定当前秘密，可识别只改引用却仍使用同一种子的候选。
        bool::from(
            self.fingerprint_for(reference)
                .as_bytes()
                .ct_eq(digest.as_bytes()),
        )
    }

    fn fingerprint_for(&self, reference: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(b"studiolo-totp-seed-v1\0");
        hash.update((reference.len() as u64).to_be_bytes());
        hash.update(reference.as_bytes());
        hash.update(self.secret.as_slice());
        crate::session::secrets::hex(&hash.finalize())
    }
}

impl fmt::Debug for TotpSeed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TotpSeed([REDACTED])")
    }
}

/// 六位 ASCII 数字验证码；保留前导零且不提供序列化、克隆或显示入口。
pub struct TotpCode(Zeroizing<String>);

impl TotpCode {
    /// 接收恰好六位 ASCII 数字；无论通过与否，输入均由零化容器持有。
    pub fn parse(value: String) -> Result<Self, SessionError> {
        let value = Zeroizing::new(value);
        if value.len() != 6 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(SessionError::Rejected);
        }
        Ok(Self(value))
    }

    pub(super) fn bytes(&self) -> &[u8] {
        self.0.as_bytes()
    }
}

impl fmt::Debug for TotpCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TotpCode([REDACTED])")
    }
}
