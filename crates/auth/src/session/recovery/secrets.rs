//! 恢复码只交付域分离派生值；原随机输入与本实例拥有的字符串在释放时零化。

use crate::session::{SessionError, SessionRandom, secrets as session_secrets};
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::Zeroizing;

/// 绑定批次与槽位的 128 位恢复秘密；不可克隆、序列化或隐式显示。
pub struct RecoveryCode {
    encoded: Zeroizing<String>,
    generation: i64,
    slot: u8,
}

impl RecoveryCode {
    /// 只接收固定 55 字节的小写规范编码；输入无论有效与否都由零化容器持有。
    pub fn parse(value: String) -> Result<Self, SessionError> {
        let encoded = Zeroizing::new(value);
        if encoded.len() != 55
            || !encoded.is_ascii()
            || !encoded.starts_with("r1.")
            || &encoded[19..20] != "."
            || &encoded[22..23] != "."
            || !session_secrets::valid_sequence(&encoded[3..19])
            || !session_secrets::lower_hex(&encoded[20..22])
            || !session_secrets::lower_hex(&encoded[23..])
        {
            return Err(SessionError::Rejected);
        }
        let generation =
            i64::from_str_radix(&encoded[3..19], 16).map_err(|_| SessionError::Rejected)?;
        let slot = u8::from_str_radix(&encoded[20..22], 16).map_err(|_| SessionError::Rejected)?;
        if slot > 9 {
            return Err(SessionError::Rejected);
        }
        Ok(Self {
            encoded,
            generation,
            slot,
        })
    }

    /// 仅供可信秘密交付层使用；不得记录、发送到聊天或写入普通档案。
    #[must_use]
    pub fn expose_secret(&self) -> &str {
        &self.encoded
    }

    pub(super) fn generation(&self) -> i64 {
        self.generation
    }
    pub(super) fn slot(&self) -> u8 {
        self.slot
    }

    pub(super) fn digest(&self, store: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(b"studiolo-recovery-code-v1\0");
        hash.update((store.len() as u64).to_be_bytes());
        hash.update(store.as_bytes());
        hash.update(self.encoded.as_bytes());
        session_secrets::hex(&hash.finalize())
    }
}

impl fmt::Debug for RecoveryCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryCode([REDACTED])")
    }
}

/// 原换发调用一次性交付的十枚恢复码；本类型不提供数据库重读或克隆能力。
pub struct RecoveryCodes(Vec<RecoveryCode>);

impl RecoveryCodes {
    /// 借用当前持有的原码；调用者自行保留的副本不受本容器零化保证覆盖。
    #[must_use]
    pub fn codes(&self) -> &[RecoveryCode] {
        &self.0
    }
}

impl fmt::Debug for RecoveryCodes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RecoveryCodes([REDACTED])")
    }
}

pub(super) struct Generated {
    pub codes: RecoveryCodes,
    pub digests: Vec<String>,
    pub seed_digests: Vec<([u8; 32], [u8; 32])>,
}

pub(super) fn generate(
    random: &dyn SessionRandom,
    store: &str,
    generation: i64,
) -> Result<Generated, SessionError> {
    if generation <= 0 || store.len() != 32 || !session_secrets::lower_hex(store) {
        return Err(SessionError::CorruptStore);
    }
    let mut codes = Vec::with_capacity(10);
    let mut digests = Vec::with_capacity(10);
    let mut seed_digests: Vec<([u8; 32], [u8; 32])> = Vec::with_capacity(10);
    for slot in 0..10 {
        let mut seed = Zeroizing::new([0_u8; 32]);
        random.fill(&mut *seed)?;
        let encoded_seed = Zeroizing::new(session_secrets::hex(&*seed));
        let token = seed_digest(b"studiolo-session-v1\0", &encoded_seed);
        let csrf = seed_digest(b"studiolo-csrf-v1\0", &encoded_seed);
        if seed_digests
            .iter()
            .any(|(prior, _)| session_secrets::equal(prior, &token))
        {
            return Err(SessionError::RandomCollision);
        }
        let code = RecoveryCode::parse(derive(&seed, store, generation, slot)?)?;
        digests.push(code.digest(store));
        codes.push(code);
        seed_digests.push((token, csrf));
    }
    Ok(Generated {
        codes: RecoveryCodes(codes),
        digests,
        seed_digests,
    })
}

fn derive(seed: &[u8; 32], store: &str, generation: i64, slot: u8) -> Result<String, SessionError> {
    let mut info = [0_u8; 25];
    // store 已由调用方核为 32 位小写十六进制，定长切分没有余数。
    let (pairs, _) = store.as_bytes().as_chunks::<2>();
    for (index, pair) in pairs.iter().enumerate() {
        let nibble = |byte: u8| {
            if byte <= b'9' {
                byte - b'0'
            } else {
                byte - b'a' + 10
            }
        };
        info[index] = nibble(pair[0]) * 16 + nibble(pair[1]);
    }
    info[16..24].copy_from_slice(&generation.to_be_bytes());
    info[24] = slot;
    let mut output = Zeroizing::new([0_u8; 16]);
    hkdf::Hkdf::<sha2_legacy::Sha256>::new(Some(b"studiolo-recovery-v1\0"), seed)
        .expand(&info, &mut *output)
        .map_err(|_| SessionError::CorruptStore)?;
    let encoded = Zeroizing::new(session_secrets::hex(&*output));
    Ok(format!(
        "r1.{generation:016x}.{slot:02x}.{}",
        encoded.as_str()
    ))
}

fn seed_digest(domain: &[u8], seed: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(seed.as_bytes());
    hash.finalize().into()
}

#[cfg(test)]
#[path = "secrets_tests.rs"]
mod tests;
