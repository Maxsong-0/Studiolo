//! 配置、消息与链接边界；不合法输入不触发网络。

use super::{super::*, support::*};
use crate::alert::AlertSeverity;
use std::collections::BTreeMap;

#[test]
fn 缺主通道仍走备用而全部缺配置不访问网络() {
    let main = Server::new(Reply::Disconnect);
    let backup = Server::new(Reply::json(200, r#"{"code":200}"#));
    for missing in [TOKEN, USER] {
        let mut settings = values();
        settings.remove(missing);
        settings.remove(LINK);
        transport(&main, &backup, settings)
            .deliver(&message(AlertSeverity::Warning, "A01"))
            .expect("缺主通道用备用");
    }
    assert_eq!(main.calls(), 0);
    assert!(!backup.request().body.contains("url"));
    let sender = transport(&main, &backup, BTreeMap::new());
    assert!(!sender.configured());
    assert_eq!(
        sender.deliver(&message(AlertSeverity::Warning, "A01")),
        Err(TransportError::NotConfigured)
    );
    assert_eq!(backup.calls(), 2);
}

#[test]
fn 不合法密钥链接和代号均在外发前拒绝() {
    let main = Server::new(Reply::Disconnect);
    let backup = Server::new(Reply::Disconnect);
    for key in [TOKEN, USER, DEVICE] {
        for bad in [
            String::new(),
            "a b".into(),
            "a\nb".into(),
            "a\u{2003}b".into(),
            "s".repeat(1025),
        ] {
            let mut settings = values();
            settings.insert(key, bad);
            assert_eq!(
                transport(&main, &backup, settings)
                    .deliver(&message(AlertSeverity::Warning, "A01")),
                Err(TransportError::Malformed)
            );
        }
    }
    let links = [
        "",
        "http://example.test/",
        "https://user:pass@example.test/",
        "https://@example.test/",
        "https://example.test/?secret=x",
        "https://example.test/#secret",
        "https://example.test/\n",
        " https://example.test/",
        "https:\\example.test/",
    ];
    for bad in links {
        let mut settings = values();
        settings.insert(LINK, bad.into());
        assert_eq!(
            transport(&main, &backup, settings).deliver(&message(AlertSeverity::Warning, "A01")),
            Err(TransportError::Malformed)
        );
    }
    let codes = [
        "",
        "a b",
        "a/b",
        "a\nb",
        "中文",
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    ];
    for bad in codes {
        assert_eq!(
            transport(&main, &backup, values()).deliver(&message(AlertSeverity::Warning, bad)),
            Err(TransportError::Malformed)
        );
    }
    let sender = PushTransport::from_lookup(|_| Err(TransportError::Malformed));
    assert_eq!(
        sender.deliver(&message(AlertSeverity::Warning, "A01")),
        Err(TransportError::Malformed)
    );
    assert_eq!((main.calls(), backup.calls()), (0, 0));
}

#[test]
fn 合法边界与缺备用配置不妨碍主通道接受() {
    let body = serde_json::json!({"status": 1, "receipt": "r".repeat(256)}).to_string();
    let main = Server::new(Reply::json(200, &body));
    let backup = Server::new(Reply::Disconnect);
    let mut settings = values();
    settings.insert(TOKEN, "t".repeat(1024));
    settings.insert(USER, "u".repeat(1024));
    settings.remove(DEVICE);
    settings.remove(LINK);
    let code = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    transport(&main, &backup, settings)
        .deliver(&message(AlertSeverity::Critical, code))
        .expect("边界值合法");
    assert!(!main.request().body.contains("url="));
    assert_eq!(backup.calls(), 0);
}

#[test]
fn 重定向不跟随且错误与调试输出无秘密() {
    let sink = Server::new(Reply::json(200, r#"{"status":1}"#));
    let main = Server::new(Reply::Redirect(sink.url.clone()));
    let backup = Server::new(Reply::json(
        400,
        "fixture-token fixture-device fixture-user",
    ));
    let sender = transport(&main, &backup, values());
    assert_eq!(format!("{sender:?}"), "PushTransport { configured: true }");
    let error = sender
        .deliver(&message(AlertSeverity::Warning, "A01"))
        .expect_err("两路拒收");
    let rendered = format!("{sender:?} {error:?} {error}");
    for secret in [
        "fixture-token",
        "fixture-user",
        "fixture-device",
        "example.test",
        "http:",
        "receipt",
    ] {
        assert!(!rendered.contains(secret));
    }
    assert_eq!((sink.calls(), backup.calls()), (0, 1));
}
