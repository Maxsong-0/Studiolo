//! 公开标准向量与纯窗口规则；固定虚构秘密，不涉及真实因子或工具链。

use super::*;
use crate::session::SessionError;

fn seed() -> TotpSeed {
    TotpSeed::new("fixture.seed-1".to_owned(), *b"12345678901234567890").unwrap()
}

fn code(seed: &TotpSeed, step: u64) -> TotpCode {
    TotpCode::parse(format!("{:06}", truncated(seed, step).unwrap() % 1_000_000)).unwrap()
}

#[test]
fn rfc4226_ten_hotp_vectors_match_six_digit_production_window() {
    // RFC 4226 附录 D 的数据，不复制参考实现。
    let expected = [
        755224, 287082, 359152, 969429, 338314, 254676, 287922, 162583, 399871, 520489,
    ];
    let seed = seed();
    for (step, expected) in expected.into_iter().enumerate() {
        let input = TotpCode::parse(format!("{expected:06}")).unwrap();
        assert_eq!(truncated(&seed, step as u64).unwrap() % 1_000_000, expected);
        assert_eq!(
            matching_step(&seed, &input, step as i64 * 30_000, step as i64 - 1),
            Some(step as i64)
        );
    }
}

#[test]
fn rfc6238_sha1_six_vectors_include_eight_digit_leading_zero_and_after_2038() {
    // RFC 6238 附录 B：八位输出仅在测试中构造，生产解析与匹配固定六位。
    let expected = [
        (59_i64, "94287082"),
        (1_111_111_109, "07081804"),
        (1_111_111_111, "14050471"),
        (1_234_567_890, "89005924"),
        (2_000_000_000, "69279037"),
        (20_000_000_000, "65353130"),
    ];
    let seed = seed();
    for (seconds, expected) in expected {
        let step = seconds / 30;
        assert_eq!(
            format!(
                "{:08}",
                truncated(&seed, step as u64).unwrap() % 100_000_000
            ),
            expected
        );
        let input = TotpCode::parse(expected[2..].to_owned()).unwrap();
        assert_eq!(
            matching_step(&seed, &input, seconds * 1_000, -1),
            Some(step)
        );
        assert_eq!(
            TotpCode::parse(expected.to_owned()).unwrap_err(),
            SessionError::Rejected
        );
    }
}

#[test]
fn window_is_exactly_previous_current_next_and_used_steps_never_repeat() {
    let seed = seed();
    for step in [4, 5, 6] {
        let input = code(&seed, step);
        assert_eq!(matching_step(&seed, &input, 150_999, 3), Some(step as i64));
        assert_eq!(matching_step(&seed, &input, 150_999, step as i64), None);
    }
    for step in [3, 7] {
        assert_eq!(matching_step(&seed, &code(&seed, step), 150_999, -1), None);
    }
    assert_eq!(
        matching_step(&seed, &code(&seed, 6), 150_999, i64::MAX),
        None
    );
}

#[test]
fn zero_step_negative_time_and_largest_millisecond_time_do_not_wrap() {
    let seed = seed();
    let zero = code(&seed, 0);
    assert_eq!(matching_step(&seed, &zero, -1, -1), None);
    assert_eq!(matching_step(&seed, &zero, i64::MIN, -1), None);
    assert_eq!(matching_step(&seed, &zero, 0, -1), Some(0));
    assert_eq!(matching_step(&seed, &zero, 29_999, -1), Some(0));
    assert_eq!(matching_step(&seed, &zero, 30_000, -1), Some(0));
    assert_eq!(matching_step(&seed, &zero, 60_000, -1), None);
    let step = i64::MAX / 30_000;
    assert_eq!(
        matching_step(&seed, &code(&seed, step as u64), i64::MAX, -1),
        Some(step)
    );
}

#[test]
fn equal_candidate_codes_choose_latest_but_any_consumed_match_denies_all() {
    // 固定折叠输入覆盖罕见同码冲突，不在测试中穷举种子或数百万时间步。
    let candidates = [
        (10, Choice::from(1)),
        (11, Choice::from(0)),
        (12, Choice::from(1)),
    ];
    assert_eq!(select_step(candidates, 9), Some(12));
    assert_eq!(select_step(candidates, 10), None);
    assert_eq!(select_step(candidates, 12), None);
    assert_eq!(select_step([(10, Choice::from(0)); 3], -1), None);
}

#[test]
fn code_parser_preserves_leading_zeroes_and_rejects_non_ascii_or_other_lengths() {
    let code = TotpCode::parse("000042".to_owned()).unwrap();
    assert_eq!(code.bytes(), b"000042");
    for invalid in [
        "",
        "12345",
        "1234567",
        " 12345",
        "12345\n",
        "12345\t",
        "12.345",
        "１２３４５６",
        "٠١٢٣٤٥",
    ] {
        assert_eq!(
            TotpCode::parse(invalid.to_owned()).unwrap_err(),
            SessionError::Rejected
        );
    }
}

#[test]
fn hex_seed_uses_the_same_reference_and_known_secret_as_direct_configuration() {
    let decoded = TotpSeed::from_hex(
        "fixture.seed-1".into(),
        b"3132333435363738393031323334353637383930",
    )
    .unwrap();
    assert_eq!(decoded.reference(), seed().reference());
    assert_eq!(decoded.bytes(), b"12345678901234567890");
    assert_eq!(decoded.fingerprint(), seed().fingerprint());
    assert_eq!(format!("{decoded:?}"), "TotpSeed([REDACTED])");
    let upper_nibbles = TotpSeed::from_hex("fixture".into(), &[b'f'; 40]).unwrap();
    assert_eq!(upper_nibbles.bytes(), &[255; 20]);
}

#[test]
fn hex_seed_rejects_noncanonical_encoding_without_normalizing_it() {
    for encoded in [
        Vec::new(),
        vec![b'0'; 39],
        vec![b'0'; 41],
        vec![b'A'; 40],
        vec![b'g'; 40],
        vec![255; 40],
        vec![0; 40],
        format!("{}\n", "0".repeat(40)).into_bytes(),
        format!("0x{}", "0".repeat(40)).into_bytes(),
        format!(" {}", "0".repeat(39)).into_bytes(),
    ] {
        assert_eq!(
            TotpSeed::from_hex("fixture".into(), &encoded).unwrap_err(),
            SessionError::Rejected
        );
    }
}

#[test]
fn hex_seed_keeps_reference_validation_and_encoding_failure_precedence() {
    assert!(TotpSeed::from_hex("a".repeat(64), &[b'0'; 40]).is_ok());
    for reference in ["".into(), "a".repeat(65), "../seed".into(), "种子".into()] {
        assert_eq!(
            TotpSeed::from_hex(reference, &[b'0'; 40]).unwrap_err(),
            SessionError::Configuration
        );
    }
    assert_eq!(
        TotpSeed::from_hex("../seed".into(), b"invalid").unwrap_err(),
        SessionError::Rejected
    );
}

#[test]
fn seed_reference_is_an_opaque_ascii_identifier_and_binding_changes_fingerprint() {
    let seed = seed();
    assert_eq!(seed.reference(), "fixture.seed-1");
    assert_eq!(seed.bytes(), b"12345678901234567890");
    assert_eq!(seed.fingerprint().len(), 64);
    assert!(
        seed.fingerprint()
            .bytes()
            .all(|value| value.is_ascii_digit() || (b'a'..=b'f').contains(&value))
    );
    assert_eq!(seed.fingerprint(), self::seed().fingerprint());
    assert_ne!(
        seed.fingerprint(),
        TotpSeed::new("fixture.seed-2".to_owned(), *seed.bytes())
            .unwrap()
            .fingerprint()
    );
    assert_ne!(
        seed.fingerprint(),
        TotpSeed::new(seed.reference().to_owned(), [7; 20])
            .unwrap()
            .fingerprint()
    );
    for invalid in [
        "".to_owned(),
        "a".repeat(65),
        "dir/seed".to_owned(),
        "dir\\seed".to_owned(),
        " seed".to_owned(),
        "种子".to_owned(),
        "seed\n".to_owned(),
    ] {
        assert_eq!(
            TotpSeed::new(invalid, [0; 20]).unwrap_err(),
            SessionError::Configuration
        );
    }
    assert!(TotpSeed::new("A_z.0-".repeat(10), [0; 20]).is_ok());
}

#[test]
fn binding_a_seed_wipes_only_the_copy_it_took_and_never_the_caller_buffer() {
    // 固定虚构字节；构造函数按值收下一份拷贝，清的只能是它自己那一份。
    let source = [3_u8; 20];
    let bound = TotpSeed::new("fixture.seed-3".to_owned(), source).unwrap();
    assert_eq!(source, [3_u8; 20], "调用方自己的缓冲区不得被构造函数改写");
    assert_eq!(bound.bytes(), &[3_u8; 20], "绑定的秘密必须与输入一致");
    // 解码路径与按值路径必须绑定同一个秘密，改写复制方式不改变行为。
    let decoded = TotpSeed::from_hex("fixture.seed-3".to_owned(), &[b'3'; 40]).unwrap();
    assert_eq!(decoded.bytes(), &[0x33_u8; 20]);
    assert_eq!(
        TotpSeed::new("fixture.seed-3".to_owned(), [0x33_u8; 20])
            .unwrap()
            .fingerprint(),
        decoded.fingerprint()
    );
}

#[test]
fn secret_debug_never_displays_reference_seed_or_otp() {
    assert_eq!(format!("{:?}", seed()), "TotpSeed([REDACTED])");
    let code = TotpCode::parse("000042".to_owned()).unwrap();
    assert_eq!(format!("{code:?}"), "TotpCode([REDACTED])");
}
