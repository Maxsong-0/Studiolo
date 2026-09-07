//! 显式通道控制审计交错；超时仅防测试挂死，不用 sleep 推测线程进度。

use super::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, AuditRecord};

const WAIT: Duration = Duration::from_secs(10);

#[test]
fn audit_wait_past_idle_activates_before_absolute_and_starts_idle_at_activation() {
    let h = Harness::new(10, 100);
    let clock = h.clock.clone();
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.issue" && event.phase() == AuditPhase::Outcome {
            clock.set(1_015_000);
        }
    }));

    let issued = h.manager.issue(&identity()).unwrap();
    assert_eq!(issued.session.state, SessionState::Active);
    assert_eq!(issued.session.created_at_ms, 1_000_000);
    assert_eq!(issued.session.last_seen_at_ms, 1_015_000);
    assert_eq!(issued.session.absolute_expires_at_ms, 1_100_000);
    assert_eq!(
        scalar(&h.manager, "SELECT activated_at FROM sessions"),
        1_015_000
    );

    // 中间不调用 check，以免成功读取再次推进 last_seen；精确检验激活后的十秒边界。
    h.clock.set(1_025_000);
    assert_eq!(
        h.manager.check(&identity(), &issued.token).unwrap_err(),
        SessionError::Rejected
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM sessions WHERE state='expired'"
        ),
        1
    );
}

#[test]
fn late_denial_ack_cannot_regress_receipt_or_drop_concurrent_increment() {
    let h = Harness::new(10, 100);
    let unknown = SessionToken::from_hex("d".repeat(64)).unwrap();
    assert_eq!(
        h.manager.check(&identity(), &unknown).unwrap_err(),
        SessionError::Rejected
    );

    let (entered_tx, entered_rx) = mpsc::sync_channel(2);
    let (early_tx, early_rx) = mpsc::sync_channel(1);
    let (late_tx, late_rx) = mpsc::sync_channel(1);
    let early_rx = Mutex::new(early_rx);
    let late_rx = Mutex::new(late_rx);
    let arrival = AtomicUsize::new(0);
    *h.audit.hook.lock().unwrap() = Some(Arc::new(move |event| {
        if event.action() == "session.denied"
            && event.payload().as_value()["through_count"].as_i64() == Some(1)
        {
            let index = arrival.fetch_add(1, Ordering::SeqCst);
            assert!(index < 2);
            entered_tx.send(index).unwrap();
            let resume = if index == 0 { &early_rx } else { &late_rx };
            resume.lock().unwrap().recv_timeout(WAIT).unwrap();
        }
    }));

    let managers = [h.reopen(10, 100), h.reopen(10, 100)];
    let (finished_tx, finished_rx) = mpsc::sync_channel(2);
    let workers: Vec<_> = managers
        .into_iter()
        .map(|manager| {
            let finished = finished_tx.clone();
            thread::spawn(move || {
                let result = manager.flush_audit(1);
                finished.send(()).unwrap();
                result
            })
        })
        .collect();
    let mut entered = [
        entered_rx.recv_timeout(WAIT).unwrap(),
        entered_rx.recv_timeout(WAIT).unwrap(),
    ];
    entered.sort();
    assert_eq!(entered, [0, 1]);

    // 两个补记者均已取得 through=1 的同一快照，此时增加两次拒绝。
    for _ in 0..2 {
        assert_eq!(
            h.manager.check(&identity(), &unknown).unwrap_err(),
            SessionError::Rejected
        );
    }
    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=2"),
        3
    );
    early_tx.send(()).unwrap();
    finished_rx.recv_timeout(WAIT).unwrap();
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT delivered FROM auth_denials WHERE kind=2"
        ),
        1
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT through_count FROM audit_outbox WHERE bucket=2"
        ),
        3
    );

    // 先把新增快照交付至3，再释放旧快照的晚到ack，能直接检出回执/计数回退。
    assert_eq!(h.manager.flush_audit(1).unwrap(), 1);
    let newest_receipt = denial_receipt(&h.manager);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT delivered FROM auth_denials WHERE kind=2"
        ),
        3
    );
    late_tx.send(()).unwrap();
    finished_rx.recv_timeout(WAIT).unwrap();
    for worker in workers {
        assert_eq!(worker.join().unwrap().unwrap(), 1);
    }
    *h.audit.hook.lock().unwrap() = None;

    assert_eq!(
        scalar(&h.manager, "SELECT total FROM auth_denials WHERE kind=2"),
        3
    );
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT delivered FROM auth_denials WHERE kind=2"
        ),
        3
    );
    assert_eq!(denial_receipt(&h.manager), newest_receipt);
    assert_eq!(
        scalar(
            &h.manager,
            "SELECT count(*) FROM audit_outbox WHERE bucket=2"
        ),
        0
    );
    assert_eq!(h.manager.flush_audit(256).unwrap(), 0);

    let records = h
        .audit
        .query(&AuditFilter::default().with_action("session.denied"))
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(
        records[0].event().payload().as_value()["after_count"].as_i64(),
        Some(0)
    );
    assert_eq!(
        records[0].event().payload().as_value()["through_count"].as_i64(),
        Some(1)
    );
    assert_eq!(
        records[1].event().payload().as_value()["after_count"].as_i64(),
        Some(1)
    );
    assert_eq!(
        records[1].event().payload().as_value()["through_count"].as_i64(),
        Some(3)
    );
    assert_eq!(
        serde_json::from_str::<AuditRecord>(&newest_receipt).unwrap(),
        records[1]
    );
    assert_eq!(scalar(&h.manager, "SELECT count(*) FROM sessions"), 0);
}

fn denial_receipt(manager: &SessionManager) -> String {
    manager
        .database
        .connection
        .lock()
        .unwrap()
        .query_row(
            "SELECT last_receipt FROM auth_denials WHERE kind=2",
            [],
            |row| row.get(0),
        )
        .unwrap()
}
