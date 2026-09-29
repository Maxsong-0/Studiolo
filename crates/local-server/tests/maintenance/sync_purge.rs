//! `sync-purge-residue` 契约：只清「上次核验排除里有、当前审批里没有、headless 最近一次核对仍排除」的
//! 文件夹残留，链接一律拒绝；删除前后各在权威审计账本记一条 `sync.purge-residue`（Intent／Outcome），
//! 账本不可用即拒绝、一个字节不删。夹具在 `sync.rs`。

use super::super::{MaintenanceError, options::Command};
use super::sync::{Sync, private_dir};
use serde_json::{Value, json};
use std::{fs, os::unix::fs::symlink};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, SqliteAuditLedger};

/// 权威账本里 `sync.purge-residue` 的全部记录：（阶段，事件标识，载荷），按追加顺序。
fn purge_audit(s: &Sync) -> Vec<(AuditPhase, String, Value)> {
    let ledger = SqliteAuditLedger::open_existing(&s.root().join("audit")).unwrap();
    let filter = AuditFilter::default().with_action("sync.purge-residue");
    let records = ledger.query(&filter).unwrap();
    records
        .iter()
        .map(|record| {
            let event = record.event();
            let payload = event.payload().as_value().clone();
            (event.phase(), event.event_id().to_string(), payload)
        })
        .collect()
}

#[test]
fn purge_refuses_folders_that_are_not_clearable_residue() {
    let s = Sync::new(true);
    s.mark_verified_excluded(&["School", "日记"]);
    private_dir(&s.view().join("School"));
    fs::write(s.view().join("School/a.md"), "残留").unwrap();
    // 没在上次核验的排除集合里。
    assert!(matches!(
        s.run(Command::SyncPurgeResidue, Some("Archive")),
        Err(MaintenanceError::SyncResidue)
    ));
    // 仍在当前审批的排除清单里：headless 自己不会往里写，也轮不到本命令删。
    assert!(matches!(
        s.run(Command::SyncPurgeResidue, Some("日记")),
        Err(MaintenanceError::SyncResidue)
    ));
    assert!(s.view().join("School/a.md").is_file());
    assert!(purge_audit(&s).is_empty());
}

#[test]
fn purge_refuses_folders_that_headless_no_longer_excludes() {
    // 错序：headless 最近一次核对已不再排除 School，残留一清就可能被下一次 RunOnce 当新内容上传。
    let s = Sync::new(true);
    s.mark_plane_state(&["School"], &[]);
    private_dir(&s.view().join("School"));
    fs::write(s.view().join("School/a.md"), "残留").unwrap();
    assert!(matches!(
        s.run(Command::SyncPurgeResidue, Some("School")),
        Err(MaintenanceError::SyncResidueHeadless)
    ));
    assert!(s.view().join("School/a.md").is_file());
    assert!(purge_audit(&s).is_empty());
}

#[test]
fn purge_without_an_audit_ledger_refuses_and_deletes_nothing() {
    let s = Sync::without_audit();
    s.mark_verified_excluded(&["School"]);
    private_dir(&s.view().join("School"));
    fs::write(s.view().join("School/a.md"), "残留").unwrap();
    assert!(matches!(
        s.run(Command::SyncPurgeResidue, Some("School")),
        Err(MaintenanceError::Audit)
    ));
    assert!(s.view().join("School/a.md").is_file());
}

#[test]
fn purge_refuses_links_and_deletes_nothing() {
    let s = Sync::new(true);
    s.mark_verified_excluded(&["School"]);
    private_dir(&s.view().join("School"));
    private_dir(&s.view().join("School/sub"));
    fs::write(s.view().join("School/a.md"), "残留").unwrap();
    symlink("a.md", s.view().join("School/sub/link.md")).unwrap();
    assert!(matches!(
        s.run(Command::SyncPurgeResidue, Some("School")),
        Err(MaintenanceError::SyncResidueUnsafe)
    ));
    assert!(s.view().join("School/a.md").is_file());
    assert!(s.view().join("School/sub/link.md").is_symlink());
    // 文件夹本身是链接也拒绝。
    private_dir(&s.root().join("elsewhere"));
    symlink(s.root().join("elsewhere"), s.view().join("Linked")).unwrap();
    s.mark_verified_excluded(&["Linked", "School"]);
    assert!(matches!(
        s.run(Command::SyncPurgeResidue, Some("Linked")),
        Err(MaintenanceError::SyncResidueUnsafe)
    ));
    assert!(s.root().join("elsewhere").is_dir());
}

#[test]
fn purge_removes_the_residue_tree_reports_exact_counts_and_audits_both_phases() {
    let s = Sync::new(true);
    s.mark_verified_excluded(&["School"]);
    private_dir(&s.view().join("School"));
    private_dir(&s.view().join("School/sub"));
    private_dir(&s.view().join("School/sub/deeper"));
    fs::write(s.view().join("School/a.md"), "残留").unwrap();
    fs::write(s.view().join("School/sub/b.md"), "残留").unwrap();
    fs::write(s.view().join("School/sub/deeper/c.txt"), "残留").unwrap();
    private_dir(&s.view().join("笔记"));
    fs::write(s.view().join("笔记/keep.md"), "不动").unwrap();
    let report = s.run(Command::SyncPurgeResidue, Some("School")).unwrap();
    assert_eq!(report["command"], "sync-purge-residue");
    assert_eq!(report["folder"], "School");
    assert_eq!(report["removed_files"], 3);
    assert_eq!(report["removed_directories"], 3);
    assert!(!s.view().join("School").exists());
    assert!(s.view().join("笔记/keep.md").is_file());
    // 删除前 Intent 记计划数，删除后 Outcome 记实删数与整趟成功；两条共用一个事件标识。
    let audit = purge_audit(&s);
    let planned = json!({"schema_version":1,"folder":"School","files":3,"directories":3});
    let mut applied = planned.clone();
    applied["applied"] = json!(true);
    assert_eq!(audit.len(), 2);
    assert_eq!((audit[0].0, &audit[0].2), (AuditPhase::Intent, &planned));
    assert_eq!((audit[1].0, &audit[1].2), (AuditPhase::Outcome, &applied));
    assert_eq!(audit[0].1, audit[1].1);
    // 已经清干净：再来一次没有残留可清，也不再记审计。
    let again = s.run(Command::SyncPurgeResidue, Some("School"));
    assert!(
        matches!(again, Err(MaintenanceError::SyncResidue)),
        "{again:?}"
    );
    assert_eq!(purge_audit(&s).len(), 2);
}
