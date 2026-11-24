use super::*;
use sha2::{Digest, Sha256};

/// 空输入的 SHA-256，取自公开的标准值，不由被测函数算出。
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

#[test]
fn sha256_text_is_exactly_64_lowercase_hex_characters() {
    assert!(is_sha256_hex(EMPTY_SHA256));
    assert!(is_sha256_hex(&"0123456789abcdef".repeat(4)));
    for rejected in [
        String::new(),
        "a".repeat(63),
        "a".repeat(65),
        "AB".repeat(32),
        format!("{}g", "a".repeat(63)),
        format!("{} ", "a".repeat(63)),
        // 64 个字节但不是 64 个 ASCII 字符。
        "é".repeat(32),
    ] {
        assert!(!is_sha256_hex(&rejected), "{rejected:?}");
    }
}

#[test]
fn hex_writes_two_lowercase_digits_per_byte() {
    assert_eq!(hex(&[]), "");
    assert_eq!(hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    assert_eq!(hex(&Sha256::digest(b"")), EMPTY_SHA256);
}
