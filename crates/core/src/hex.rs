//! 字节与小写十六进制文本互转、SHA-256 文本形状的判断：全仓只在这一处定义（T-246），各包改调这里，
//! 不再各写一份（架构守卫 `hex_single_definition` 钉住）。

use sha2::{Digest as _, Sha256};

const DIGITS: &[u8; 16] = b"0123456789abcdef";

/// 字节写成小写十六进制，每字节两位。
///
/// 按长度一次分配、查表逐位写，不经 `format!`：auth 拿它编码恢复码、TOTP 种子，结果包在 `Zeroizing` 里；
/// 逐字节 `format!` 收集每字节一个临时 `String`、收集途中还会扩容，会在堆上留下没清零的秘密碎片。
#[must_use]
pub fn to_lower_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for &byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    output
}

/// 全部是 `0-9a-f`（空串也算）；长度由调用方另核。
#[must_use]
pub fn is_lower_hex(text: impl AsRef<[u8]>) -> bool {
    text.as_ref().iter().all(|&byte| digit(byte).is_some())
}

/// SHA-256 的文本写法：恰好 64 个小写十六进制字符。
#[must_use]
pub fn is_sha256_hex(text: impl AsRef<[u8]>) -> bool {
    let text = text.as_ref();
    text.len() == 64 && is_lower_hex(text)
}

/// 恰好 `2N` 个小写十六进制字符解成 `N` 字节；长度、大小写、字符不对都返回 `None`。
///
/// 不用 `u8::from_str_radix`：它收大写，还收前导 `+`（`"+a"` 解成 10）。
#[must_use]
pub fn from_lower_hex<const N: usize>(text: impl AsRef<[u8]>) -> Option<[u8; N]> {
    let text = text.as_ref();
    if text.len() != N.checked_mul(2)? {
        return None;
    }
    let mut output = [0u8; N];
    for (slot, pair) in output.iter_mut().zip(text.chunks_exact(2)) {
        let [high, low] = pair else { return None };
        *slot = (digit(*high)? << 4) | digit(*low)?;
    }
    Some(output)
}

/// 普通 SHA-256（不带域分隔前缀）的小写十六进制。带前缀的 hasher 自己算摘要，只把「编」那一步交给 [`to_lower_hex`]。
#[must_use]
pub fn sha256_hex(bytes: impl AsRef<[u8]>) -> String {
    to_lower_hex(&Sha256::digest(bytes.as_ref()))
}

const fn digit(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}
