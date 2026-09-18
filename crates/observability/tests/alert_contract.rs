//! 带外告警通道与调度的行为／失败／恢复三类契约。
//!
//! 全程使用本地假 transport；**仓库里没有任何真实收件地址或凭据**。

mod support;

use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase};
use studiolo_observability::alert::{
    ALERT_AUDIT_ACTION, AlertDispatcher, AlertMessage, AlertSeverity, AlertTransport,
    DispatchOutcome, EnvironmentSmtpTransport, IdempotencyKey, SMTP_TO_VARIABLE, SMTP_URL_VARIABLE,
    TransportError,
};
use support::{BrokenLedger, ScriptedTransport, working_ledger};

fn message(seed: &str) -> AlertMessage {
    AlertMessage::new(
        IdempotencyKey::derive("test", &[seed]),
        AlertSeverity::Critical,
        "deadman-switch",
        "约定周期内没有收到活性回执",
    )
    .expect("固定测试正文必须合法")
}

#[test]
fn 通道可用时一次尝试即送达并记住幂等键() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always_ok();
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(outcome, DispatchOutcome::Delivered { attempts: 1 });
    assert_eq!(transport.calls(), 1);
    assert_eq!(dispatcher.remembered(), 1);
}

#[test]
fn 同一把幂等键再次调度时不再重复发送() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always_ok();
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");
    let alert = message("a");

    let first = dispatcher.dispatch(&ledger, &transport, &alert, 1);
    let second = dispatcher.dispatch(&ledger, &transport, &alert, 2);

    assert_eq!(first, DispatchOutcome::Delivered { attempts: 1 });
    assert_eq!(second, DispatchOutcome::AlreadyDelivered);
    assert_eq!(transport.calls(), 1);
}

#[test]
fn 幂等键只由域标签与要素派生因而跨进程稳定() {
    let left = IdempotencyKey::derive("deadman-switch", &["no-receipt", "7"]);
    let right = IdempotencyKey::derive("deadman-switch", &["no-receipt", "7"]);
    let other = IdempotencyKey::derive("deadman-switch", &["no-receipt", "8"]);

    assert_eq!(left, right);
    assert_ne!(left, other);
    assert_eq!(left.as_str().len(), 32);
}

#[test]
fn 通道超时时按上限有界重试并留下固定失败分类() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always(TransportError::Timeout);
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(
        outcome,
        DispatchOutcome::Failed {
            classification: "timeout",
            attempts: 3,
        }
    );
    assert_eq!(transport.calls(), 3);
    assert_eq!(dispatcher.remembered(), 0);
}

#[test]
fn 通道不可用时重试到上限后如实报告不可用() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always(TransportError::Unavailable);
    let mut dispatcher = AlertDispatcher::new(2, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(
        outcome,
        DispatchOutcome::Failed {
            classification: "unavailable",
            attempts: 2,
        }
    );
}

#[test]
fn 对端明确拒收时立即失败不做无谓重试() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always(TransportError::Rejected);
    let mut dispatcher = AlertDispatcher::new(5, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(
        outcome,
        DispatchOutcome::Failed {
            classification: "rejected",
            attempts: 1,
        }
    );
    assert_eq!(transport.calls(), 1);
}

#[test]
fn 发送端未配置时返回未配置而不是静默成功() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always(TransportError::NotConfigured);
    let mut dispatcher = AlertDispatcher::new(5, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(outcome, DispatchOutcome::NotConfigured);
    assert_eq!(transport.calls(), 1);
    assert_eq!(dispatcher.remembered(), 0);
}

#[test]
fn 审计不可用时一次都不发送并报告审计不可用() {
    let ledger = BrokenLedger;
    let transport = ScriptedTransport::always_ok();
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(outcome, DispatchOutcome::AuditUnavailable);
    assert_eq!(transport.calls(), 0);
}

#[test]
fn 每次调度都留下固定动作的两个phase审计且payload不含正文() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::always_ok();
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");

    let _ = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    let records = ledger
        .query(&AuditFilter::default())
        .expect("内存账本必须可查询");
    assert_eq!(records.len(), 2);
    assert!(
        records
            .iter()
            .all(|record| record.event().action() == ALERT_AUDIT_ACTION)
    );
    assert_eq!(records[0].event().phase(), AuditPhase::Intent);
    assert_eq!(records[1].event().phase(), AuditPhase::Outcome);
    let payload = records[1].event().payload().as_value().to_string();
    assert!(payload.contains("deadman-switch"));
    assert!(!payload.contains("活性回执"));
    assert!(!payload.contains(SMTP_URL_VARIABLE));
}

#[test]
fn 不同调度轮次各自留痕因而重复调度不会撞上幂等冲突() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::new(vec![Err(TransportError::Timeout), Ok(())]);
    let mut dispatcher = AlertDispatcher::new(1, 16).expect("固定参数必须合法");
    let alert = message("a");

    let first = dispatcher.dispatch(&ledger, &transport, &alert, 1);
    let second = dispatcher.dispatch(&ledger, &transport, &alert, 2);

    assert_eq!(
        first,
        DispatchOutcome::Failed {
            classification: "timeout",
            attempts: 1,
        }
    );
    assert_eq!(second, DispatchOutcome::Delivered { attempts: 1 });
    assert_eq!(
        ledger
            .query(&AuditFilter::default())
            .expect("内存账本必须可查询")
            .len(),
        4
    );
}

#[test]
fn 通道从超时恢复之后同一情节仍能真正送达() {
    let ledger = working_ledger();
    let transport = ScriptedTransport::new(vec![
        Err(TransportError::Timeout),
        Err(TransportError::Timeout),
        Ok(()),
    ]);
    let mut dispatcher = AlertDispatcher::new(3, 16).expect("固定参数必须合法");

    let outcome = dispatcher.dispatch(&ledger, &transport, &message("a"), 1);

    assert_eq!(outcome, DispatchOutcome::Delivered { attempts: 3 });
}

#[test]
fn 去重容量有界因而旧键被淘汰后不会无界增长() {
    let mut dispatcher = AlertDispatcher::new(1, 2).expect("固定参数必须合法");

    for seed in ["a", "b", "c"] {
        dispatcher.remember(IdempotencyKey::derive("test", &[seed]));
    }

    assert_eq!(dispatcher.remembered(), 2);
    assert!(!dispatcher.contains(&IdempotencyKey::derive("test", &["a"])));
    assert!(dispatcher.contains(&IdempotencyKey::derive("test", &["c"])));
}

#[test]
fn 重试次数或容量为零的调度器参数被直接拒绝() {
    assert!(AlertDispatcher::new(0, 16).is_err());
    assert!(AlertDispatcher::new(1, 0).is_err());
    assert!(AlertDispatcher::new(2_000, 16).is_err());
}

#[test]
fn 告警正文含路径或邮箱形状时构造直接失败() {
    let key = IdempotencyKey::derive("test", &["a"]);
    assert!(AlertMessage::new(key.clone(), AlertSeverity::Warning, "x", "/var/backup").is_err());
    assert!(AlertMessage::new(key, AlertSeverity::Warning, "x", "a@b").is_err());
}

#[test]
fn 环境里缺少收件配置时真实smtp通道报未配置而不是伪装成功() {
    // 本用例不设置任何环境变量，也不读取它们的值，只看"在不在"。
    let transport = EnvironmentSmtpTransport::from_environment();
    if transport.configured() {
        // 本机确实注入了发送端配置：仍然不得伪装成功。
        assert_eq!(
            transport.deliver(&message("a")),
            Err(TransportError::Unavailable)
        );
    } else {
        assert_eq!(
            transport.deliver(&message("a")),
            Err(TransportError::NotConfigured)
        );
    }
}

#[test]
fn 真实smtp通道的调试输出只暴露是否已配置不含任何值() {
    let rendered = format!("{:?}", EnvironmentSmtpTransport::from_environment());

    assert!(rendered.contains("configured"));
    assert!(!rendered.contains(SMTP_URL_VARIABLE));
    assert!(!rendered.contains(SMTP_TO_VARIABLE));
    assert!(!rendered.contains('@'));
}
