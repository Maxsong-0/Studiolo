//! 旧 Delete/Restore 配置无 Rename，也必须先补原 Outcome 再完成真实 owner 重开。
use super::support::OutcomeFailure;
use crate::{
    editing::refresh,
    file_operations::{PATH, delete, post},
    fixture::Fixture,
};
use serde_json::json;
use std::{fs, os::unix::fs::MetadataExt, sync::Arc};
use studiolo_audit_ledger::AuditPhase;

fn disk(f: &Fixture, restored: bool) -> Option<(u64, u64)> {
    let path = f.http.root.join("vault/a.md");
    if restored {
        assert_eq!(fs::read(&path).unwrap(), f.expected.content);
        let metadata = fs::symlink_metadata(path).unwrap();
        assert!(metadata.is_file());
        Some((metadata.dev(), metadata.ino()))
    } else {
        assert_eq!(
            fs::symlink_metadata(path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        None
    }
}

#[tokio::test]
async fn delete_and_restore_pending_sqlite_outcomes_reopen_without_rename_or_reexecution() {
    for restore in [false, true] {
        let mut f = Fixture::file_operations(None).await;
        let issued = f.http.issue().await;
        assert_eq!(
            f.get(PATH, &issued.cookie, 200).await["rename_enabled"],
            false
        );
        let input = if restore {
            let removed = post(&f, &issued, &delete(&f, "legacy-before-restore-01"), 200).await;
            assert_eq!(removed["receipt"]["sequence"], "1");
            assert_eq!(disk(&f, false), None);
            refresh(&f, &issued).await;
            json!({"kind":"restore_tombstone","vault_uuid":f.expected.vault_uuid,
                "path":"a.md","tombstone_id":removed["receipt"]["tombstone_id"],
                "idempotency_key":"legacy-outcome-reopen-01"})
        } else {
            delete(&f, "legacy-outcome-reopen-01")
        };
        let sequence = if restore { 2_u64 } else { 1_u64 };
        let action = if restore {
            "recovery.page_restore"
        } else {
            "vault.mutation"
        };
        let failing = Arc::new(OutcomeFailure::open(&f.http.audit));
        f.reopen_file_operations_with_audit(Some(failing)).await;
        assert_eq!(
            post(&f, &issued, &input, 503).await["code"],
            "outcome_unknown"
        );
        let pending = f.journal();
        assert_ne!(pending["rename_enabled"], true);
        assert_eq!(pending["file_operation"]["phase"], "finalized_await_audit");
        assert_eq!(pending["committed_through"], sequence);
        assert_eq!(pending["next_sequence"], sequence + 1);
        let fixed = &pending["file_operation"]["terminal"]["receipt"];
        assert_eq!(fixed["outcome"], "committed");
        assert_eq!(fixed["sequence"], sequence);
        let identity = disk(&f, restore);
        let before = f.http.events(action);
        assert_eq!(before.len(), 1, "真实 SQLite 只有当前操作的原 Intent");
        assert_eq!(before[0].event().phase(), AuditPhase::Intent);
        assert_eq!(
            f.http.events("vault.mutation").len(),
            if restore { 2 } else { 1 }
        );
        assert_eq!(
            f.http.events("recovery.page_restore").len(),
            if restore { 1 } else { 0 }
        );

        // 同一私有配置、同一真实 SQLite；只重新装配可用 ledger，不能重建或丢弃旧 ACK。
        f.reopen_file_operations().await;
        assert_eq!(
            f.get(PATH, &issued.cookie, 200).await["rename_enabled"],
            false
        );
        let replay = post(&f, &issued, &input, 200).await;
        assert_eq!(replay["status"], "committed");
        assert_eq!(replay["receipt"]["sequence"], sequence.to_string());
        assert_eq!(replay["receipt"]["operation_id"], fixed["operation_id"]);
        assert_eq!(replay["receipt"]["outcome"], "committed");
        assert_eq!(replay["receipt"]["effects"].as_array().unwrap().len(), 1);
        assert_eq!(post(&f, &issued, &input, 200).await, replay);
        assert_eq!(
            disk(&f, restore),
            identity,
            "重放不得重新安装或更换 Restore inode"
        );
        let closed = f.journal();
        assert!(closed["file_operation"].is_null());
        assert_eq!(closed["committed_through"], sequence);
        assert_eq!(closed["next_sequence"], pending["next_sequence"]);
        assert_ne!(closed["rename_enabled"], true);
        let after = f.http.events(action);
        assert_eq!(after.len(), 2, "重开与重复核对只补原固定 Outcome");
        assert_eq!(after[0].event(), before[0].event());
        assert_eq!(after[1].event().phase(), AuditPhase::Outcome);
        assert_eq!(
            after[1].event().payload().as_value()["file_operation"]["operation_id"],
            fixed["operation_id"]
        );
        assert_eq!(f.http.events("vault.mutation").len(), 2);
        assert_eq!(
            f.http.events("recovery.page_restore").len(),
            if restore { 2 } else { 0 }
        );
        f.close().await;
    }
}
