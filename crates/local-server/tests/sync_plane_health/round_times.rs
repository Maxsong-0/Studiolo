//! 一轮的两个时刻（BUG-048、BUG-049）：开始时刻喂给驱动器，也是同步状态回执「对上」「守卫证明」的时刻
//! （契约 `sync-status-receipt.md`）；驱动器返回之后另取结束时刻，给 owner 活性回执和最近闭合的一批
//! （契约 `health-sync-bridge.md` 第 2、3 节）。终态未知、冻结的轮次不让「最近闭合的一批」前移。
use super::*;
use crate::sync_plane::run_and_record;
use std::cell::Cell;

/// 依次报出给定时刻的假时钟；一轮读的次数超过给的个数就是实现多读了时钟，直接判红。
fn ticks(times: &[i64]) -> impl Fn() -> i64 + '_ {
    let next = Cell::new(0);
    move || {
        let index = next.get();
        next.set(index + 1);
        *times.get(index).expect("一轮只该读两次时钟")
    }
}

/// 最近一批的（序号，时刻）。
fn batch(health: &PlaneHealth) -> Option<(u64, i64)> {
    health
        .bridge()
        .last_batch
        .map(|stamp| (stamp.sequence, stamp.unix_seconds))
}

fn receipt_at(health: &PlaneHealth) -> Option<Instant> {
    health.liveness().last_receipt
}

#[test]
fn 活性回执取驱动器返回之后的时刻而对上和守卫证明仍取这一轮开始() {
    let health = PlaneHealth::new(None);

    run_and_record(&health, ticks(&[5_000, 5_070]), |now| {
        assert_eq!(now, 5_000, "驱动器拿的是这一轮开始的时刻");
        health.record_delete_gate(0);
        Ok(RoundOutcome::Idle)
    });

    assert_eq!(receipt_at(&health), Some(Instant::from_unix_seconds(5_070)));
    let receipt = health.status_receipt(5_071);
    assert_eq!(receipt.last_aligned_at, Some(5_000));
    assert_eq!(
        (receipt.deletes_blocked, receipt.deletes_checked_at),
        (Some(0), Some(5_000))
    );
}

/// 契约第 3 节：无论那轮成功与否，回执都是那一轮结束的时刻。
#[test]
fn 失败或停住的轮次活性回执同样取结束时刻() {
    for outcome in [
        Err(PlaneError::Transport),
        Err(PlaneError::Observation),
        unknown(3),
        blocked(3),
        Ok(RoundOutcome::Skipped),
    ] {
        let health = after_clean_batch();

        run_and_record(&health, ticks(&[6_000, 6_090]), |_| outcome);

        assert_eq!(
            receipt_at(&health),
            Some(Instant::from_unix_seconds(6_090)),
            "{outcome:?}"
        );
    }
}

#[test]
fn 一批闭合的时刻取那一轮结束时且正常正文说的就是它() {
    let health = PlaneHealth::new(None);
    fully_reported(&health, 7);

    run_and_record(&health, ticks(&[10_000, 10_045]), |_| settled(7));

    assert_eq!(batch(&health), Some((7, 10_045)));
    let section = bridge_section(Some(&health.bridge())).expect("固定正文必须可构造");
    assert_eq!(section.state(), HealthState::Normal);
    assert!(
        section.detail().starts_with("第 7 批于 10045 闭合；"),
        "{}",
        section.detail()
    );
}

#[test]
fn 终态未知和冻结的轮次不让最近闭合的一批前移而下一批闭合才前移() {
    let health = after_clean_batch();
    assert_eq!(batch(&health), Some((1, 10_000)));

    for (at, outcome) in [
        (10_300, unknown(2)),
        (10_600, unknown(2)),
        (10_900, blocked(2)),
    ] {
        health.record_round(span(at), &outcome);
        assert_eq!(batch(&health), Some((1, 10_000)), "{outcome:?}");
    }

    fully_reported(&health, 2);
    health.record_round(span(11_200), &settled(2));
    assert_eq!(batch(&health), Some((2, 11_200)));
}

/// 刚重启头一批就卡住：本进程没有一批闭合过就说不出，不拿卡住的那批冒充「最近闭合」。
#[test]
fn 还没有一批闭合过时停住的轮次不盖最近一批() {
    let health = PlaneHealth::new(None);

    health.record_round(span(10_000), &unknown(11));
    health.record_round(span(10_300), &blocked(11));

    assert_eq!(health.bridge().last_batch, None);
}

/// 冻结的批次即使别的证据都没问题，正文也不能说它闭合了。
#[test]
fn 冻结的批次不会被正文说成已闭合() {
    let health = after_clean_batch();
    fully_reported(&health, 2);

    health.record_round(span(10_300), &blocked(2));

    let section = bridge_section(Some(&health.bridge())).expect("固定正文必须可构造");
    assert!(
        !section.detail().contains("第 2 批于"),
        "{}",
        section.detail()
    );
}
