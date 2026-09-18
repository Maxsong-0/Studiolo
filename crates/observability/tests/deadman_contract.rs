//! 死人开关状态机的行为／失败／恢复三类契约。

use studiolo_observability::alert::{
    DeadmanDecision, DeadmanPolicy, DeadmanReason, DeadmanState, evaluate,
};

fn policy() -> DeadmanPolicy {
    DeadmanPolicy::new(600, 60, 3_600, 3).expect("固定测试参数必须合法")
}

fn step(state: DeadmanState, now: i64) -> (DeadmanState, DeadmanDecision) {
    evaluate(policy(), state, now).expect("固定正文必须可构造")
}

fn reason(decision: &DeadmanDecision) -> Option<DeadmanReason> {
    match decision {
        DeadmanDecision::Fire { reason, .. } => Some(*reason),
        _ => None,
    }
}

#[test]
fn 回执在约定周期内时判定为正常() {
    let state = DeadmanState::started_at(1_000).with_receipt(1_000);

    let (_, decision) = step(state, 1_500);

    assert_eq!(decision, DeadmanDecision::Healthy);
}

#[test]
fn 刚好等于约定周期时仍然算正常不提前触发() {
    let state = DeadmanState::started_at(1_000).with_receipt(1_000);

    let (_, decision) = step(state, 1_600);

    assert_eq!(decision, DeadmanDecision::Healthy);
}

#[test]
fn 超期但仍在去抖窗口内时本轮不报() {
    let state = DeadmanState::started_at(1_000).with_receipt(1_000);

    let (_, decision) = step(state, 1_630);

    assert_eq!(
        decision,
        DeadmanDecision::Debouncing {
            overdue_seconds: 30
        }
    );
}

#[test]
fn 超期越过去抖窗口后触发缺回执告警并推进调度轮次() {
    let state = DeadmanState::started_at(1_000).with_receipt(1_000);

    let (next, decision) = step(state, 1_700);

    assert_eq!(reason(&decision), Some(DeadmanReason::NoReceipt));
    assert_eq!(next.last_alert, Some(1_700));
    assert_eq!(next.dispatch_round, 1);
    assert_eq!(next.episode, 1);
}

#[test]
fn 从未收到回执时以状态起点为锚而不是当作刚刚收到() {
    let state = DeadmanState::started_at(1_000);

    let (_, decision) = step(state, 1_700);

    assert_eq!(reason(&decision), Some(DeadmanReason::NoReceipt));
}

#[test]
fn 本地时钟回退时报时间证据不可信并且仍然触发() {
    let state = DeadmanState::started_at(5_000).with_receipt(5_000);

    let (next, decision) = step(state, 4_000);

    assert_eq!(reason(&decision), Some(DeadmanReason::ClockRolledBack));
    assert_eq!(next.last_observed, 4_000);
    assert_eq!(next.dispatch_round, 1);
}

#[test]
fn 时钟回退不受冷却期压制仍然立即触发() {
    let mut state = DeadmanState::started_at(1_000).with_receipt(1_000);
    let fired = step(state, 1_700);
    state = fired.0;
    assert!(reason(&fired.1).is_some());

    let (_, decision) = step(state, 1_100);

    assert_eq!(reason(&decision), Some(DeadmanReason::ClockRolledBack));
}

#[test]
fn 时间差越界时按时间证据不可信处理而不是静默跳过() {
    let state = DeadmanState {
        last_receipt: Some(i64::MIN),
        last_observed: i64::MIN,
        last_alert: None,
        suppressed_since_alert: 0,
        episode: 0,
        dispatch_round: 0,
    };

    let (_, decision) = step(state, i64::MAX);

    assert_eq!(reason(&decision), Some(DeadmanReason::ClockRolledBack));
}

#[test]
fn 冷却期内连续故障先被压制但压制次数被记录下来() {
    let mut state = DeadmanState::started_at(1_000).with_receipt(1_000);
    state = step(state, 1_700).0;

    let (next, decision) = step(state, 1_800);

    assert!(matches!(
        decision,
        DeadmanDecision::Suppressed {
            suppressed_count: 1,
            ..
        }
    ));
    assert_eq!(next.suppressed_since_alert, 1);
}

#[test]
fn 冷却期内压制达到上限后强制升级再报一次而不是一次都不报() {
    let mut state = DeadmanState::started_at(1_000).with_receipt(1_000);
    state = step(state, 1_700).0;
    let mut suppressed = 0;
    let mut escalated = None;
    for offset in 1..=6 {
        let (next, decision) = step(state, 1_700 + offset * 100);
        state = next;
        match decision {
            DeadmanDecision::Suppressed { .. } => suppressed += 1,
            DeadmanDecision::Fire { reason, .. } => {
                escalated = Some(reason);
                break;
            }
            other => panic!("冷却期内不应出现 {other:?}"),
        }
    }

    assert_eq!(suppressed, 3);
    assert_eq!(escalated, Some(DeadmanReason::EscalatedDuringCooldown));
    assert_eq!(state.dispatch_round, 2);
}

#[test]
fn 升级告警使用新情节因而幂等键与上一条不同() {
    let mut state = DeadmanState::started_at(1_000).with_receipt(1_000);
    let first = step(state, 1_700);
    state = first.0;
    let DeadmanDecision::Fire { alert: first, .. } = first.1 else {
        panic!("第一次必须触发");
    };
    let mut escalated = None;
    for offset in 1..=6 {
        let (next, decision) = step(state, 1_700 + offset * 100);
        state = next;
        if let DeadmanDecision::Fire { alert, .. } = decision {
            escalated = Some(alert);
            break;
        }
    }

    let escalated = escalated.expect("压制到上限后必须升级再报");

    assert_ne!(first.key(), escalated.key());
}

#[test]
fn 冷却结束后同一情节复用同一把幂等键以便调度器去重() {
    let mut state = DeadmanState::started_at(1_000).with_receipt(1_000);
    let first = step(state, 1_700);
    state = first.0;
    let DeadmanDecision::Fire { alert: first, .. } = first.1 else {
        panic!("第一次必须触发");
    };
    state.suppressed_since_alert = 0;

    let (_, decision) = step(state, 1_700 + 3_600);
    let DeadmanDecision::Fire { alert: second, .. } = decision else {
        panic!("冷却结束后必须再报");
    };

    assert_eq!(first.key(), second.key());
}

#[test]
fn 收到新回执后死人开关恢复正常并清空压制计数() {
    let mut state = DeadmanState::started_at(1_000).with_receipt(1_000);
    state = step(state, 1_700).0;
    state = step(state, 1_800).0;
    assert_eq!(state.suppressed_since_alert, 1);

    state = state.with_receipt(1_900);
    let (next, decision) = step(state, 2_000);

    assert_eq!(decision, DeadmanDecision::Healthy);
    assert_eq!(next.suppressed_since_alert, 0);
}

#[test]
fn 周期为零或压制上限为零的参数被直接拒绝() {
    assert!(DeadmanPolicy::new(0, 60, 3_600, 3).is_err());
    assert!(DeadmanPolicy::new(600, 60, 3_600, 0).is_err());
}
