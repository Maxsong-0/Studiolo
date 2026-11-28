//! `parse_digest` 只认 64 位小写十六进制（T-246 随 Q1 收紧：以前 `u8::from_str_radix` 还收大写和前导 `+`）。

use studiolo_local_server::sync_plane::driver::evidence::{hex, parse_digest};

#[test]
fn parse_digest_accepts_exactly_64_lowercase_hex_characters() {
    assert_eq!(parse_digest(&"00".repeat(32)), Some([0; 32]));
    assert_eq!(parse_digest(&"ff".repeat(32)), Some([0xff; 32]));
    let pattern = [0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef];
    let expected: [u8; 32] = std::array::from_fn(|index| pattern[index % 8]);
    assert_eq!(parse_digest(&"0123456789abcdef".repeat(4)), Some(expected));
    assert_eq!(parse_digest(&hex(&expected)), Some(expected));
}

#[test]
fn parse_digest_rejects_uppercase_plus_signs_and_wrong_lengths() {
    for rejected in [
        "AB".repeat(32),
        format!("{}A", "a".repeat(63)),
        // `u8::from_str_radix("+a", 16)` 是 10，收紧前会被当成 `0a`。
        format!("+a{}", "0".repeat(62)),
        format!("{}g", "a".repeat(63)),
        "a".repeat(63),
        "a".repeat(65),
        String::new(),
        "é".repeat(32),
    ] {
        assert_eq!(parse_digest(&rejected), None, "{rejected:?}");
    }
}
