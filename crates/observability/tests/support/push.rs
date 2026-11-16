//! 真实 HTTP 假服务验证协议、回退、时限及线程边界。

#[path = "push/config.rs"]
mod config;
#[path = "push/support.rs"]
mod support;

use super::*;
use crate::alert::AlertSeverity;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Barrier, atomic::Ordering},
    thread,
    time::Duration,
};
use support::*;

#[test]
fn 主通道两级字段正确且成功后不访问备用() {
    for level in [AlertSeverity::Warning, AlertSeverity::Critical] {
        let main = Server::new(Reply::json(
            200,
            r#"{"status":1,"receipt":"fixture-receipt"}"#,
        ));
        let backup = Server::new(Reply::Disconnect);
        let alert = message(level, "A01_test-code");
        let receipt = transport(&main, &backup, values())
            .deliver(&alert)
            .expect("主通道接受");
        assert_eq!(receipt.key(), alert.key());
        let request = main.request();
        assert!(request.head.starts_with("POST /push HTTP/1.1"));
        assert!(
            request
                .head
                .to_ascii_lowercase()
                .contains("content-type: application/x-www-form-urlencoded")
        );
        let url =
            reqwest::Url::parse(&format!("https://example.test/?{}", request.body)).expect("表单");
        let actual: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
        let mut expected: BTreeMap<String, String> = [
            ("token", "fixture-token"),
            ("user", "fixture-user"),
            ("message", "Needs attention: code A01_test-code"),
            (
                "priority",
                if level == AlertSeverity::Critical {
                    "2"
                } else {
                    "0"
                },
            ),
            ("url", "https://example.test/status"),
        ]
        .into_iter()
        .map(|(key, value)| (key.into(), value.into()))
        .collect();
        if level == AlertSeverity::Critical {
            expected.extend([
                ("retry".into(), "60".into()),
                ("expire".into(), "3000".into()),
            ]);
        }
        assert_eq!(actual, expected);
        assert_eq!(backup.calls(), 0);
    }
}

#[test]
fn 备用两级只发代号且不归档() {
    for level in [AlertSeverity::Warning, AlertSeverity::Critical] {
        let main = Server::new(Reply::json(403, "refused"));
        let backup = Server::new(Reply::json(200, r#"{"code":200}"#));
        transport(&main, &backup, values())
            .deliver(&message(level, "A01"))
            .expect("备用接受");
        let request = backup.request();
        assert!(request.head.starts_with("POST /push HTTP/1.1"));
        assert!(
            request
                .head
                .to_ascii_lowercase()
                .contains("content-type: application/json")
        );
        let mut expected = json!({
            "device_key": "fixture-device",
            "body": "Needs attention: code A01",
            "level": if level == AlertSeverity::Critical { "critical" } else { "passive" },
            "isArchive": 0,
            "url": "https://example.test/status",
        });
        if level == AlertSeverity::Critical {
            expected["volume"] = json!(10);
            expected["call"] = json!(1);
        }
        assert_eq!(
            serde_json::from_str::<Value>(&request.body).expect("JSON 请求"),
            expected
        );
    }
}

#[test]
fn 主通道所有未确认接受的结果都尝试备用() {
    let mut replies = vec![
        Reply::json(400, "rejected"),
        Reply::json(429, "limited"),
        Reply::json(503, "unavailable"),
        Reply::json(200, "invalid"),
        Reply::json(200, r#"{"status":0}"#),
        Reply::json(200, r#"{"status":"1","receipt":"r"}"#),
        Reply::json(200, r#"{"status":1.0,"receipt":"r"}"#),
        Reply::json(200, r#"{"status":1}"#),
        Reply::json(200, r#"{"status":1,"receipt":""}"#),
        Reply::json(200, r#"{"status":1,"receipt":"bad\nreceipt"}"#),
        Reply::Disconnect,
    ];
    replies.push(Reply::json(
        200,
        &json!({"status":1,"receipt":"r".repeat(257)}).to_string(),
    ));
    for reply in replies {
        let main = Server::new(reply);
        let backup = Server::new(Reply::json(200, r#"{"code":200}"#));
        transport(&main, &backup, values())
            .deliver(&message(AlertSeverity::Critical, "A01"))
            .expect("备用接受");
        assert_eq!((main.calls(), backup.calls()), (1, 1));
    }
}

#[test]
fn 请求和响应体超时都尝试备用且双失败保留超时() {
    for reply in [Reply::Stall, Reply::BodyStall] {
        for accepted in [true, false] {
            let main = Server::new(reply.clone());
            let status = if accepted { 200 } else { 403 };
            let backup = Server::new(Reply::json(status, r#"{"code":200}"#));
            let mut sender = transport(&main, &backup, values());
            sender.http =
                http::Options::for_test(&main.url, &backup.url, Duration::from_millis(300));
            let result = sender.deliver(&message(AlertSeverity::Warning, "A01"));
            if accepted {
                assert!(result.is_ok());
            } else {
                assert_eq!(result, Err(TransportError::Timeout));
            }
            assert_eq!(backup.calls(), 1);
        }
    }
}

#[test]
fn 双失败保留可重试分类() {
    let cases = [
        (
            Reply::json(429, ""),
            Reply::json(403, ""),
            TransportError::Unavailable,
        ),
        (
            Reply::json(400, ""),
            Reply::json(500, ""),
            TransportError::Unavailable,
        ),
        (
            Reply::Disconnect,
            Reply::json(400, ""),
            TransportError::Unavailable,
        ),
        (
            Reply::json(400, ""),
            Reply::Disconnect,
            TransportError::Unavailable,
        ),
        (Reply::json(429, ""), Reply::Stall, TransportError::Timeout),
        (
            Reply::json(400, ""),
            Reply::json(403, ""),
            TransportError::Rejected,
        ),
    ];
    for (main_reply, backup_reply, expected) in cases {
        let main = Server::new(main_reply);
        let backup = Server::new(backup_reply);
        let mut sender = transport(&main, &backup, values());
        sender.http = http::Options::for_test(&main.url, &backup.url, Duration::from_millis(300));
        assert_eq!(
            sender.deliver(&message(AlertSeverity::Warning, "A01")),
            Err(expected)
        );
    }
}

#[test]
fn 回执超过预算或业务字段不对不能伪装成功() {
    let bodies = [
        "x".repeat(4097),
        "not json".into(),
        r#"{"code":500}"#.into(),
        r#"{"code":"200"}"#.into(),
        r#"{"code":200.0}"#.into(),
    ];
    for body in bodies {
        let main = Server::new(Reply::json(403, ""));
        let backup = Server::new(Reply::json(200, &body));
        assert_eq!(
            transport(&main, &backup, values()).deliver(&message(AlertSeverity::Warning, "A01")),
            Err(TransportError::Rejected)
        );
    }
    for size in [4096, 4097] {
        let body = format!("{}{{\"status\":1}}", " ".repeat(size - 12));
        let main = Server::new(Reply::json(200, &body));
        let backup = Server::new(Reply::json(403, ""));
        let result =
            transport(&main, &backup, values()).deliver(&message(AlertSeverity::Warning, "A01"));
        assert_eq!(result.is_ok(), size == 4096);
        assert_eq!(backup.calls(), usize::from(size > 4096));
    }
}

#[test]
fn 同步通道可在已有tokio运行时调用且同实例串行发送() {
    let main = Server::new(Reply::Slow);
    let backup = Server::new(Reply::Disconnect);
    let sender = transport(&main, &backup, values());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("测试运行时");
    runtime.block_on(async {
        sender
            .deliver(&message(AlertSeverity::Warning, "A01"))
            .expect("已有运行时中可调用");
    });
    let barrier = Barrier::new(4);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| {
                    barrier.wait();
                    sender.deliver(&message(AlertSeverity::Warning, "A01"))
                })
            })
            .collect();
        for handle in handles {
            assert!(handle.join().expect("发送线程").is_ok());
        }
    });
    assert_eq!(main.calls(), 5);
    assert_eq!(main.peak.load(Ordering::SeqCst), 1);
    assert_eq!(backup.calls(), 0);
}
