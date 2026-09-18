//! 行为契约：真实临时 SQLite ＋ 冻结计划的完整一轮执行。
//! 内存审计与假搬运端口不代表真实 Broker/镜像身份验收。

use super::runner::*;
use super::support::*;
use crate::executor::*;
use crate::sqlite::*;
use crate::*;
use studiolo_audit_ledger::MemoryAuditLedger;

fn prepared(store: &mut BridgeState, input: &Input) -> Plan {
    let frozen = ready(input);
    store
        .reserve(store.view().unwrap().version, input.clone())
        .unwrap();
    frozen
}

#[test]
fn a_mirror_bound_copy_is_staged_by_content_address_before_it_is_issued_and_applied() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let mut input = input(&store, 1);
    input.canonical.entries.insert(path(0), present(2));
    let plan = prepared(&mut store, &input);
    assert_eq!(plan.actions().len(), 1);
    assert_eq!(plan.actions()[0].key.target, Side::Mirror);

    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.issued, 1);
    assert_eq!(report.committed, 1);
    assert_eq!(report.unknown, 0);
    assert!(report.settled);
    assert!(report.baseline_advanced);
    assert_eq!(store.view().unwrap().baseline.version, 2);

    // 内容先按地址落库封口，然后才 Issued，再由后端安装，最后保存回执。
    let open = rig.first("open Canonical").unwrap();
    let finish = rig.first("sink.finish").unwrap();
    let issue = rig.first("journal.issue").unwrap();
    let apply = rig.first("backend.apply").unwrap();
    let record = rig.first("journal.record_outcome").unwrap();
    assert!(open < finish && finish < issue && issue < apply && apply < record);
    assert_eq!(rig.sink.finished.len(), 1);
    assert_eq!(
        rig.sink.finished[0],
        ObjectId::from_digest(version(2).sha256)
    );
    // 固定 64 KiB 帧：70000 字节正好两帧。
    let frames: Vec<String> = rig
        .lines()
        .into_iter()
        .filter(|line| line.starts_with("sink.accept"))
        .collect();
    assert_eq!(frames, vec!["sink.accept 65536", "sink.accept 4464"]);
    // 复制用的暂存对象在确认安装后交还额度。
    assert_eq!(rig.cas.usage().entries, 0);
    assert!(report.preserved.is_empty());
}

#[test]
fn a_canonical_bound_action_is_never_staged_in_the_mirror_cas() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 1);
    let mut input = input(&store, 1);
    input.mirror.entries.insert(path(0), present(2));
    let plan = prepared(&mut store, &input);
    assert_eq!(plan.actions()[0].key.target, Side::Canonical);

    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.committed, 1);
    assert_eq!(rig.backend.staged, vec![None]);
    assert_eq!(rig.cas.usage(), StageUsage::default());
    assert_eq!(rig.first("open Mirror"), None);
}

#[test]
fn a_mirror_deletion_preserves_the_target_bytes_before_the_delete_is_applied() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 50);
    let mut input = input(&store, 50);
    input.canonical.entries.insert(path(0), Presence::Missing);
    let plan = prepared(&mut store, &input);
    assert!(matches!(plan.actions()[0].change, Change::Delete { .. }));

    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.committed, 1);
    assert_eq!(report.deletion.deletions, 1);
    assert_eq!(report.deletion.baseline_live_paths, 50);
    // 目标字节在删除动作被执行之前就已按内容地址保全。
    assert!(rig.first("open Mirror").unwrap() < rig.first("backend.apply").unwrap());
    assert!(rig.first("sink.finish").unwrap() < rig.first("backend.apply").unwrap());
    assert_eq!(
        report.preserved,
        vec![ObjectId::from_digest(version(1).sha256)]
    );
    // 保全记录仍被引用，不会被当成可回收孤儿。
    assert_eq!(rig.cas.usage().entries, 1);
    assert_eq!(rig.cas.usage().orphans, 0);
    // 没有完整共同观察就只终结计划，不推进共同基线。
    assert!(report.settled);
    assert!(!report.baseline_advanced);
    assert_eq!(store.view().unwrap().baseline.version, 1);
}

#[test]
fn job_keys_follow_the_plan_sequence_and_identical_content_never_shares_one_key() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 2);
    let mut input = input(&store, 2);
    // 两条路径的源内容完全相同：内容 hash 相同，但作业键必须不同。
    input.canonical.entries.insert(path(0), present(2));
    input.canonical.entries.insert(path(1), present(2));
    let plan = prepared(&mut store, &input);
    let views = store.retry_pending(&input).unwrap();
    assert_eq!(views.len(), 2);
    assert_ne!(views[0].external_key, views[1].external_key);
    let marker = format!("-{}-", input.plan_sequence);
    assert!(views.iter().all(|view| view.external_key.contains(&marker)));

    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(2)));
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.committed, 2);
    // 同一内容只占一个内容地址，但两条动作各有自己的原键。
    assert_eq!(rig.sink.finished.len(), 2);
    assert_eq!(rig.sink.finished[0], rig.sink.finished[1]);
}

#[test]
fn an_agreement_only_plan_settles_and_advances_the_common_baseline() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 3);
    let input = input(&store, 3);
    let plan = prepared(&mut store, &input);
    assert!(plan.actions().is_empty());
    assert_eq!(plan.agreements().len(), 3);

    let mut rig = Rig::new(&input, store.observation_epoch(), Some(present(1)));
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.issued, 0);
    assert!(report.settled);
    assert!(report.baseline_advanced);
    assert_eq!(store.view().unwrap().baseline.version, 2);
    assert_eq!(rig.cas.usage(), StageUsage::default());
}

#[test]
fn five_deletions_against_fifty_surviving_paths_are_counted_as_one_batch_and_pass() {
    let fixture = Fixture::new();
    let ledger = MemoryAuditLedger::new();
    let mut store = fixture.init();
    seed(&mut store, &ledger, 50);
    let mut input = input(&store, 50);
    for index in 0..5 {
        input
            .canonical
            .entries
            .insert(path(index), Presence::Missing);
    }
    let plan = prepared(&mut store, &input);
    assert_eq!(plan.actions().len(), 5);

    let audit = audit_batch(&plan).unwrap();
    assert_eq!(audit.deletions, 5);
    assert_eq!(audit.baseline_live_paths, 50);
    assert_eq!(audit.verdict, None);

    let mut rig = Rig::new(&input, store.observation_epoch(), None);
    let report = with_store(&mut store, &ledger, &input, &plan, &mut rig).unwrap();
    assert_eq!(report.committed, 5);
    assert_eq!(report.preserved.len(), 5);
    assert!(report.settled);
    assert!(!report.baseline_advanced);
}
