//! 十六进制共用函数（T-246）两侧都钉：该认的认、该拒的拒；期望值取自公开标准值或独立写法，不由被测函数算出。
use sha2::{Digest as _, Sha256};
use studiolo_core::{from_lower_hex, is_lower_hex, is_sha256_hex, sha256_hex, to_lower_hex};

/// 空输入的 SHA-256，取自公开的标准值（FIPS 180-4 测试向量）。
const EMPTY_SHA256: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
/// `"abc"` 的 SHA-256，同一出处。
const ABC_SHA256: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

#[test]
fn to_lower_hex_matches_an_independent_two_digit_encoding_for_every_byte() {
    for byte in 0..=u8::MAX {
        assert_eq!(to_lower_hex(&[byte]), format!("{byte:02x}"), "{byte}");
    }
    let all = (0..=u8::MAX).collect::<Vec<_>>();
    let expected = all
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    assert_eq!(to_lower_hex(&all), expected);
}

#[test]
fn to_lower_hex_keeps_leading_zeros_and_maps_empty_to_empty() {
    assert_eq!(to_lower_hex(&[]), "");
    assert_eq!(to_lower_hex(&[0x00, 0x0f, 0xa0, 0xff]), "000fa0ff");
    assert_eq!(to_lower_hex(&Sha256::digest(b"")), EMPTY_SHA256);
}

#[test]
fn sha256_hex_is_plain_sha256_without_a_domain_prefix() {
    assert_eq!(sha256_hex(b""), EMPTY_SHA256);
    assert_eq!(sha256_hex(b"abc"), ABC_SHA256);
    assert_eq!(sha256_hex("abc"), ABC_SHA256);
    let owned = Vec::from("abc");
    assert_eq!(sha256_hex(owned), ABC_SHA256);
}

#[test]
fn sha256_text_is_exactly_64_lowercase_hex_characters() {
    assert!(is_sha256_hex(EMPTY_SHA256));
    assert!(is_sha256_hex("0123456789abcdef".repeat(4)));
    assert!(is_sha256_hex("0".repeat(64)));
    assert!(is_sha256_hex(ABC_SHA256.as_bytes()));
    for rejected in [
        String::new(),
        "a".repeat(63),
        "a".repeat(65),
        "AB".repeat(32),
        format!("{}A", "a".repeat(63)),
        format!("{}g", "a".repeat(63)),
        format!("{} ", "a".repeat(63)),
        format!("+{}", "a".repeat(63)),
        format!("{}\n", "a".repeat(64)),
        // 64 个字节但不是 64 个 ASCII 字符。
        "é".repeat(32),
    ] {
        assert!(!is_sha256_hex(&rejected), "{rejected:?}");
    }
}

#[test]
fn lower_hex_charset_accepts_only_0_to_9_and_a_to_f() {
    assert!(is_lower_hex(""));
    assert!(is_lower_hex("0123456789abcdef"));
    for byte in 0..=u8::MAX {
        let expected = byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
        assert_eq!(is_lower_hex([byte]), expected, "{byte}");
    }
}

#[test]
fn from_lower_hex_round_trips_and_accepts_only_exact_lowercase_pairs() {
    let all: [u8; 256] = std::array::from_fn(|index| u8::try_from(index).unwrap());
    assert_eq!(from_lower_hex::<256>(to_lower_hex(&all)), Some(all));
    assert_eq!(
        from_lower_hex::<4>("000fa0ff"),
        Some([0x00, 0x0f, 0xa0, 0xff])
    );
    assert_eq!(from_lower_hex::<0>(""), Some([]));
    let digest = from_lower_hex::<32>(EMPTY_SHA256).unwrap();
    assert_eq!(digest.as_slice(), Sha256::digest(b"").as_slice());
    for rejected in [
        // 大写、`from_str_radix` 会收的前导 `+`、非十六进制字符、空白。
        "000FA0FF",
        "000fA0ff",
        "+a0fa0ff",
        "000fa0fg",
        "000fa0f ",
        " 00fa0ff",
        // 长度不等于 2N：奇数、少一对、多一对。
        "000fa0f",
        "000fa0",
        "000fa0ff00",
        "",
    ] {
        assert_eq!(from_lower_hex::<4>(rejected), None, "{rejected:?}");
    }
    // 多字节字符凑够字节数也不认。
    assert_eq!(from_lower_hex::<2>("éé"), None);
}
