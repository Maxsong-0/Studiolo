use super::{request_ip, same_uid};
use axum::http::{HeaderMap, HeaderValue};

#[test]
fn exactly_one_ascii_bare_ip_is_required_for_every_request() {
    for text in ["192.0.2.17", "2001:db8::17", "::ffff:192.0.2.17"] {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", HeaderValue::from_str(text).unwrap());
        assert_eq!(
            request_ip(&headers).unwrap(),
            text.parse::<std::net::IpAddr>().unwrap()
        );
    }
    let invalid: &[&[u8]] = &[
        b"",
        b" ",
        b" 192.0.2.1",
        b"192.0.2.1 ",
        b"192.0.2.1\t",
        b"192.0.2.1,192.0.2.2",
        b"192.0.2.1,",
        b"192.0.2.1:443",
        b"[2001:db8::1]",
        b"[2001:db8::1]:443",
        b"fe80::1%en0",
        b"not-an-ip",
        b"\xff",
        b"192.0.2.1\xff",
    ];
    for bytes in invalid {
        let mut headers = HeaderMap::new();
        headers.insert("cf-connecting-ip", HeaderValue::from_bytes(bytes).unwrap());
        assert!(request_ip(&headers).is_err(), "不应接受 {bytes:?}");
    }
    let mut headers = HeaderMap::new();
    headers.insert("x-forwarded-for", HeaderValue::from_static("192.0.2.1"));
    assert!(request_ip(&headers).is_err());
    headers.append("cf-connecting-ip", HeaderValue::from_static("192.0.2.1"));
    headers.append("cf-connecting-ip", HeaderValue::from_static("192.0.2.1"));
    assert!(request_ip(&headers).is_err());
}

#[test]
fn uid_comparison_rejects_missing_and_different_credentials() {
    // 这是私有判断的边界证据；不同 UID 进程的端到端凭据不由此测试声称覆盖。
    assert!(same_uid(Some(501), 501));
    assert!(!same_uid(Some(502), 501));
    assert!(!same_uid(None, 501));
    assert!(!same_uid(Some(0), 501));
}
