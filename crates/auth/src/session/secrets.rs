//! v2 上下文派生秘密与旧凭证兼容；原种子不交付，摘要比较不承诺数据库恒时。

use super::{SessionError, SessionId, SessionRandom};
use sha2::{Digest, Sha256};
use std::fmt;
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

macro_rules! secret {
    ($name:ident, $prefix:literal) => {
        /// 零化容器中的 256 位不透明秘密；Debug 脱敏，不支持序列化和隐式克隆。
        pub struct $name(Zeroizing<String>);
        impl $name {
            /// 接收旧64hex或本类型v2规范编码；保留旧方法名，输入由零化容器持有。
            pub fn from_hex(value: String) -> Result<Self, SessionError> {
                let value = Zeroizing::new(value);
                if !valid_encoded(&value, $prefix) {
                    return Err(SessionError::Rejected);
                }
                Ok(Self(value))
            }
            /// 只供受信任传输层交付；调用方不得记录或持久化返回值。
            #[must_use]
            pub fn expose_secret(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([REDACTED])"))
            }
        }
    };
}

secret!(SessionToken, "s2.");
secret!(CsrfToken, "c2.");

pub(super) struct Generated {
    pub id: SessionId,
    pub token: SessionToken,
    pub csrf: CsrfToken,
    pub token_digest: [u8; 32],
    pub csrf_digest: [u8; 32],
    pub token_as_csrf: [u8; 32],
    pub csrf_as_token: [u8; 32],
    pub seed_token: [u8; 32],
    pub seed_token_as_csrf: [u8; 32],
    pub seed_csrf_as_token: [u8; 32],
    pub seed_csrf: [u8; 32],
}

pub(super) fn generate(
    random: &dyn SessionRandom,
    store_id: &str,
    issuance: i64,
) -> Result<Generated, SessionError> {
    let mut token = Zeroizing::new([0_u8; 32]);
    let mut csrf = Zeroizing::new([0_u8; 32]);
    random.fill(&mut *token)?;
    random.fill(&mut *csrf)?;
    if equal(&token[..], &csrf[..]) {
        return Err(SessionError::RandomCollision);
    }
    let token_hex = Zeroizing::new(hex(&*token));
    let csrf_hex = Zeroizing::new(hex(&*csrf));
    let seed_token = digest(b"studiolo-session-v1\0", &token_hex);
    let seed_token_as_csrf = digest(b"studiolo-csrf-v1\0", &token_hex);
    let seed_csrf_as_token = digest(b"studiolo-session-v1\0", &csrf_hex);
    let seed_csrf = digest(b"studiolo-csrf-v1\0", &csrf_hex);
    let id = SessionId::parse(format!("v2-{issuance:016x}"))?;
    let token = SessionToken::from_hex(derive(&token, store_id, issuance, 1, "s2")?)?;
    let csrf = CsrfToken::from_hex(derive(&csrf, store_id, issuance, 2, "c2")?)?;
    Ok(Generated {
        token_digest: token_digest(&token),
        csrf_digest: csrf_digest(&csrf),
        token_as_csrf: digest(b"studiolo-csrf-v1\0", token.expose_secret()),
        csrf_as_token: digest(b"studiolo-session-v1\0", csrf.expose_secret()),
        seed_token,
        seed_token_as_csrf,
        seed_csrf_as_token,
        seed_csrf,
        id,
        token,
        csrf,
    })
}

fn derive(
    seed: &[u8; 32],
    store_id: &str,
    issuance: i64,
    purpose: u8,
    prefix: &str,
) -> Result<String, SessionError> {
    if issuance <= 0 || store_id.len() != 32 || !lower_hex(store_id) {
        return Err(SessionError::CorruptStore);
    }
    let mut info = [0_u8; 25];
    for (index, pair) in store_id.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |byte: u8| {
            if byte <= b'9' {
                byte - b'0'
            } else {
                byte - b'a' + 10
            }
        };
        info[index] = nibble(pair[0]) * 16 + nibble(pair[1]);
    }
    info[16..24].copy_from_slice(&issuance.to_be_bytes());
    info[24] = purpose;
    let mut output = Zeroizing::new([0_u8; 32]);
    hkdf::Hkdf::<sha2_legacy::Sha256>::new(Some(b"studiolo-auth-v2\0"), seed)
        .expand(&info, &mut *output)
        .map_err(|_| SessionError::CorruptStore)?;
    let encoded = Zeroizing::new(hex(&*output));
    Ok(format!("{prefix}.{issuance:016x}.{}", encoded.as_str()))
}

pub(super) fn lower_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) fn valid_sequence(value: &str) -> bool {
    value.len() == 16 && lower_hex(value) && i64::from_str_radix(value, 16).is_ok_and(|v| v > 0)
}

fn valid_encoded(value: &str, prefix: &str) -> bool {
    if value.len() == 64 && lower_hex(value) {
        return true;
    }
    let Some(rest) = value.strip_prefix(prefix) else {
        return false;
    };
    let Some((sequence, secret)) = rest.split_once('.') else {
        return false;
    };
    valid_sequence(sequence) && secret.len() == 64 && lower_hex(secret)
}

pub(super) fn token_digest(token: &SessionToken) -> [u8; 32] {
    digest(b"studiolo-session-v1\0", token.expose_secret())
}

pub(super) fn csrf_digest(token: &CsrfToken) -> [u8; 32] {
    digest(b"studiolo-csrf-v1\0", token.expose_secret())
}

fn digest(domain: &[u8], secret: &str) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update(secret.as_bytes());
    hash.finalize().into()
}

pub(super) fn equal(left: &[u8], right: &[u8]) -> bool {
    bool::from(left.ct_eq(right))
}

pub(super) fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[usize::from(byte >> 4)] as char);
        output.push(DIGITS[usize::from(byte & 15)] as char);
    }
    output
}
