//! 投影队头阻塞回归（2026-09-06 T-032）：一张被人写坏的任务页只隔离它自己的任务，
//! 其他任务的投影与 Run 照常；坏任务自己的 Run 仍被它的页面挡住，事件保留待办。

use studiolo_audit_ledger::MemoryAuditLedger;
use studiolo_control_db::{ControlDb, TaskStatus};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_task_engine::{
    PersistentTaskEngine, RetryPolicy, TaskCommand, TaskEngine, TaskEngineError, TaskSpec,
};
use studiolo_vault_broker::{MemoryVaultBroker, Mutation, VaultReader, VaultWriter};

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).expect("fixture path is valid")
}

fn spec(input: VaultPath) -> TaskSpec {
    TaskSpec {
        role: "implementer".to_owned(),
        adapter: "stub".to_owned(),
        input_paths: vec![input],
        allow_web: Vec::new(),
        allow_db: Vec::new(),
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff_seconds: 0,
            retry_on: Vec::new(),
        },
        timeout: 300,
        project: "Studiolo".to_owned(),
        parent: None,
        title: Some("投影隔离".to_owned()),
        instruction: Some("总结输入并生成结果".to_owned()),
    }
}

#[test]
fn a_rejected_task_page_only_quarantines_its_own_task() {
    let vault = VaultUuid::new();
    let mut broker = MemoryVaultBroker::with_uuid(vault);
    broker
        .mutate(Mutation::create(
            path("input.md"),
            b"input".to_vec(),
            "initial",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(
        ControlDb::open_in_memory(vault).unwrap(),
        broker,
        MemoryAuditLedger::new(),
    );
    // 序号更小的坏任务排在队头，健康任务的 Draft 投影排在它后面。
    let broken = engine.create_task(spec(path("input.md")), 1).unwrap();
    let healthy = engine.create_task(spec(path("input.md")), 2).unwrap();
    let broken_page = path(&format!("ops/tasks/{broken}.md"));
    let healthy_page = path(&format!("ops/tasks/{healthy}.md"));

    // 人在投影之前手写了一张没有 frontmatter 的同名页面。
    let (db, mut broker, ledger) = engine.into_parts();
    broker
        .mutate(Mutation::create(
            broken_page.clone(),
            "# 这不是任务页\n".as_bytes().to_vec(),
            "human-authored-page",
        ))
        .unwrap();
    let mut engine = PersistentTaskEngine::new(db, broker, ledger);

    let error = engine.project_pending().unwrap_err();
    match error {
        TaskEngineError::ProjectionIncomplete {
            delivered,
            failures,
        } => {
            assert_eq!(
                delivered, 1,
                "healthy task projected despite the broken head"
            );
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].task_id, broken);
            assert!(
                failures[0].reason.contains("frontmatter"),
                "reason names the page problem: {}",
                failures[0].reason
            );
        }
        other => panic!("expected ProjectionIncomplete, got {other:?}"),
    }
    let pending = engine.control_db().pending_outbox().unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].task_id, broken);
    assert!(engine.broker().read(&healthy_page).is_ok());
    assert_eq!(
        engine.broker().read(&broken_page).unwrap().content,
        "# 这不是任务页\n".as_bytes()
    );

    // 健康任务的 Run 不受坏页影响；坏任务自己的 Run 仍被它的页面挡住。
    assert_eq!(
        engine.command(healthy, TaskCommand::Run, 3).unwrap().status,
        TaskStatus::Queued
    );
    assert!(matches!(
        engine.command(broken, TaskCommand::Run, 4),
        Err(TaskEngineError::ProjectionIncomplete { delivered: 0, ref failures })
            if failures.len() == 1 && failures[0].task_id == broken
    ));
    assert_eq!(
        engine.task(broken).unwrap().unwrap().status,
        TaskStatus::Draft
    );
    // 待办里只剩坏任务的原始 Draft 事件与健康任务 Run 新入队的 Queued 投影；
    // 坏任务没有被静默丢弃，健康任务的推进没有被它拖住。
    let pending = engine.control_db().pending_outbox().unwrap();
    assert_eq!(
        pending
            .iter()
            .map(|event| event.task_id)
            .collect::<Vec<_>>(),
        vec![broken, healthy]
    );
}
