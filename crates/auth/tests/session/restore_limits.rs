//! 有界计数与 epoch 极限夹具，拒绝时逐表比较，无隐藏清库路径。
use super::*;
use serde_json::json;

#[test]
fn old_schema_and_unenrolled_factor_fail_closed() {
    let h = Harness::new(3600, 7200);
    assert_eq!(
        h.manager
            .invalidate_restored_credentials("old")
            .unwrap_err(),
        SessionError::MigrationRequired
    );
    SessionManager::migrate_v2(
        &h.root,
        config(3600, 7200),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    )
    .unwrap();
    let manager = SessionManager::migrate_v3(
        &h.root,
        config(3600, 7200),
        h.audit.clone(),
        h.clock.clone(),
        h.random.clone(),
    )
    .unwrap();
    let before = whole(&manager);
    assert_eq!(
        manager
            .invalidate_restored_credentials("empty")
            .unwrap_err(),
        SessionError::FactorUnavailable
    );
    assert_eq!(whole(&manager), before);
}

#[test]
fn restore_id_maximum_fits_original_event_id_budget() {
    let h = RecoveryHarness::new(3600, 7200);
    let id = "a".repeat(55);
    assert!(
        h.h.manager
            .invalidate_restored_credentials(&id)
            .unwrap()
            .audit_confirmed
    );
    assert_eq!(
        scalar(
            &h.h.manager,
            "SELECT max(length(event_id)) FROM audit_outbox"
        ),
        96
    );
}

#[test]
fn full_security_outbox_preserves_every_row_and_retained_restore_counts_toward_budget() {
    let h = RecoveryHarness::new(3600, 7200);
    h.h.manager.invalidate_restored_credentials("kept").unwrap();
    h.h.manager.maintain_retention().unwrap();
    h.h.manager.database.transaction(|db|{
        let store=schema::store_id(db)?;
        let used:i64=db.query_row("SELECT count(*) FROM audit_outbox WHERE bucket IS NULL",[],|r|r.get(0)).unwrap();
        assert_eq!(used,2);
        for index in 0..((schema::MAX_SECURITY_EVENTS-used)/2) {
            let target=format!("cap{index}");
            for phase in [AuditPhase::Intent,AuditPhase::Outcome] {
                let event=events::event(&format!("{store}:revoke:{target}"),phase,"session.revoked",
                    json!({"scope":"one","target":target,"revoked":1,"identity_version":2,"effective":"already_revoked"}))?;
                outbox::enqueue(db,&event,"revoke",None)?;
            }
        }
        Ok(())
    }).unwrap();
    let before = whole(&h.h.manager);
    assert_eq!(
        h.h.manager
            .invalidate_restored_credentials("full")
            .unwrap_err(),
        SessionError::Capacity
    );
    assert_eq!(whole(&h.h.manager), before);
    // 已存在的 ID 无需容量；重确认只复用原两条，不再推进身份版本。
    assert_eq!(
        h.h.manager
            .invalidate_restored_credentials("kept")
            .unwrap()
            .epoch,
        2
    );
}

#[test]
fn sequence_and_epoch_overflow_refuse_without_mutation() {
    for epoch_overflow in [false, true] {
        let h = RecoveryHarness::new(3600, 7200);
        if epoch_overflow {
            h.h.manager.invalidate_restored_credentials("base").unwrap();
            h.h.manager.maintain_retention().unwrap();
            let (encoded, store): (String, String) = {
                let db = h.h.manager.database.connection.lock().unwrap();
                db.query_row(
                    "SELECT checkpoint_json,store_id FROM auth_retention_checkpoint,auth_meta",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .unwrap()
            };
            // 仅虚构已认证检查点；保留结构的规范字段次序，模拟合法达到最大 epoch。
            let changed =
                encoded.replace("\"epoch_base\":2", &format!("\"epoch_base\":{}", i64::MAX));
            assert_ne!(encoded, changed);
            let value: serde_json::Value = serde_json::from_str(&changed).unwrap();
            let event=events::event(&format!("{store}:compact:{}",value["sequence"]),AuditPhase::Outcome,"session.compacted",
                json!({"manifest_hash":value["manifest_hash"],"reclaimed_sessions":value["reclaimed_sessions"],
                    "epoch_base":value["epoch_base"],"issuance_floor":value["issuance_floor"],
                    "event_sequence_barrier":value["event_sequence_barrier"],"previous_sequence":value["previous_sequence"],
                    "previous_manifest":value["previous_manifest"],"cleanup_authorized":true})).unwrap();
            let record = MemoryAuditLedger::default().append(event.clone()).unwrap();
            let receipt = outbox::receipt(&record, &event).unwrap();
            let db = h.h.manager.database.connection.lock().unwrap();
            db.execute(
                "UPDATE auth_retention_checkpoint SET checkpoint_json=?1,receipt_json=?2",
                rusqlite::params![changed, receipt],
            )
            .unwrap();
            db.execute("UPDATE auth_meta SET epoch=?1", [i64::MAX])
                .unwrap();
        } else {
            execute(
                &h.h.manager,
                "UPDATE auth_meta SET next_sequence=9223372036854775807",
            );
        }
        let before = whole(&h.h.manager);
        assert_eq!(
            h.h.manager
                .invalidate_restored_credentials("overflow")
                .unwrap_err(),
            SessionError::Capacity
        );
        assert_eq!(whole(&h.h.manager), before);
    }
}

#[test]
fn restore_open_and_invalidation_never_sample_clock_or_rewrite_session_deadlines() {
    struct CountClock {
        calls: std::sync::atomic::AtomicUsize,
        now: i64,
    }
    impl SessionClock for CountClock {
        fn now_unix_ms(&self) -> Result<i64, SessionError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.now)
        }
    }
    // 机器时钟超前会错误到期；库时钟地板超前会错误拒绝，两种都不应影响离线撤权。
    for (now, floor) in [(NEXT + 20_000_000, NEXT), (NEXT, NEXT + 20_000_000)] {
        let h = RecoveryHarness::new(3600, 7200);
        let (issued, _) = h.ready();
        execute(
            &h.h.manager,
            &format!("UPDATE auth_meta SET clock_floor={floor}"),
        );
        let original = whole(&h.h.manager);
        let deadlines = rows(
            &h.h.manager,
            "SELECT id,created,last_seen,absolute_expiry,access_expiry,activated_at FROM sessions ORDER BY id",
        );
        let clock = Arc::new(CountClock {
            calls: std::sync::atomic::AtomicUsize::new(0),
            now,
        });
        let manager = SessionManager::open_existing_for_restore(
            &h.h.root,
            config(3600, 7200),
            h.h.audit.clone(),
            clock.clone(),
            h.h.random.clone(),
        )
        .unwrap();
        assert_eq!(whole(&manager), original);
        let result = manager
            .invalidate_restored_credentials("clock-free")
            .unwrap();
        assert!(result.audit_confirmed);
        assert_eq!(result.revoked_sessions, 1);
        assert_eq!(clock.calls.load(Ordering::SeqCst), 0);
        assert_eq!(scalar(&manager, "SELECT clock_floor FROM auth_meta"), floor);
        assert_eq!(
            scalar(
                &manager,
                "SELECT count(*) FROM sessions WHERE state='revoked'"
            ),
            1
        );
        assert_eq!(
            rows(
                &manager,
                "SELECT id,created,last_seen,absolute_expiry,access_expiry,activated_at FROM sessions ORDER BY id"
            ),
            deadlines
        );
        // 专用装配不削弱后续普通会话动作：它仍执行 run 并采样时钟。
        assert!(manager.check(&identity(), &issued.token).is_err());
        assert_eq!(clock.calls.load(Ordering::SeqCst), 1);
    }
}
