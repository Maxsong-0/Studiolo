//! 使用真实文件系统、同一 SQLite 文件与 Stub 验收 M0 垂直链。

use studiolo::M0Workbench;
use studiolo_audit_ledger::{AuditFilter, AuditLedger as _, AuditPhase};
use studiolo_control_db::TaskStatus;
use studiolo_core::{Revision, VaultPath};
use studiolo_runner_adapter::{RunnerAdapter as _, RunnerObservation};
use studiolo_task_engine::{
    ClaimRequest, PersistentTaskEngine, RetryPolicy, TaskCommand, TaskEngine as _, TaskSpec,
};
use studiolo_vault_broker::{FsVaultBroker, Mutation, VaultReader as _, VaultWriter as _};
use studiolo_vault_index::{InMemoryVaultIndex, MarkdownDocument, VaultIndex as _};
use tempfile::TempDir;

fn path(value: &str) -> VaultPath {
    VaultPath::parse(value).unwrap()
}

fn spec() -> TaskSpec {
    TaskSpec {
        role: "implementer".into(),
        adapter: "stub".into(),
        input_paths: vec![path("input.md")],
        allow_web: vec![],
        allow_db: vec![],
        retry_policy: RetryPolicy {
            max_attempts: 1,
            backoff_seconds: 0,
            retry_on: vec![],
        },
        timeout: 300,
        project: "Studiolo".into(),
        parent: None,
        title: Some("验证 M0".into()),
        instruction: Some("根据 [[input]] 生成结果。".into()),
    }
}

fn fixture() -> (TempDir, M0Workbench) {
    let root = TempDir::new().unwrap();
    let vault = root.path().join("canonical");
    let data = root.path().join("data");
    let mut broker = FsVaultBroker::open(&vault, &data).unwrap();
    broker
        .mutate(Mutation::create(
            path("input.md"),
            "# 输入\n\nM0 输入。\n".as_bytes().to_vec(),
            "seed-input",
        ))
        .unwrap();
    drop(broker);
    let workbench = M0Workbench::open(&vault, &data).unwrap();
    (root, workbench)
}

fn author_edit(
    mut workbench: M0Workbench,
    page: &VaultPath,
    edit: impl FnOnce(String) -> String,
) -> M0Workbench {
    let (db, mut broker, ledger) = workbench.tasks.into_parts();
    let before = broker.read(page).unwrap();
    let after = edit(String::from_utf8(before.content).unwrap());
    broker
        .mutate(Mutation::replace(
            page.clone(),
            after.into_bytes(),
            before.revision,
            format!("author-{}", before.revision),
        ))
        .unwrap();
    workbench.tasks = PersistentTaskEngine::new(db, broker, ledger);
    workbench
}

#[test]
fn vertical_chain_preserves_author_fields_and_publishes_verified_manifest() {
    let (root, mut workbench) = fixture();
    let task = workbench
        .tasks
        .create_task(spec(), 1_788_480_000_000)
        .unwrap();
    workbench.tasks.project_pending().unwrap();
    let page = path(&format!("ops/tasks/{task}.md"));
    workbench = author_edit(workbench, &page, |text| {
        text.replacen("---\n", "---\ncustom: {keep: [1, 2]} # 作者备注\n", 1)
            + "\n手写补充，不可被投影覆盖。\n"
    });
    workbench.tasks.command(task, TaskCommand::Run, 10).unwrap();
    // queued 投影会改变页面 revision，但不能把自己的四个投影字段误判为新输入。
    workbench.tasks.project_pending().unwrap();
    let claimed = workbench
        .tasks
        .claim(ClaimRequest {
            worker_id: "m0-test".into(),
            now_ms: 11,
            lease_until_ms: 1011,
        })
        .unwrap()
        .unwrap();
    assert_eq!(claimed.run_spec.inputs.len(), 2);
    assert!(
        claimed
            .run_spec
            .inputs
            .iter()
            .any(|input| input.path == page
                && String::from_utf8_lossy(&input.content).contains("手写补充"))
    );
    let started = workbench.runner.start(&claimed.run_spec).unwrap();
    workbench
        .tasks
        .attach_runner(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            started.runner_ref,
            12,
        )
        .unwrap();
    let RunnerObservation::Completed(report) = workbench.runner.observe(&started.handle).unwrap()
    else {
        panic!("Stub 应同步产出结果")
    };
    let first = workbench
        .tasks
        .report("m0-vertical-report", report.clone(), 13)
        .unwrap();
    let replay = workbench
        .tasks
        .report("m0-vertical-report", report, 14)
        .unwrap();
    assert_eq!(first.receipt, replay.receipt);
    assert!(replay.replayed);
    assert_eq!(
        first.receipt.manifest_path.as_str(),
        format!(
            "sources/attempts/{task}/{}/manifest.json",
            claimed.run_spec.attempt_id
        )
    );
    let broker = workbench.tasks.broker();
    let manifest = broker.read(&first.receipt.manifest_path).unwrap();
    assert_eq!(manifest.revision, first.receipt.manifest_revision);
    for artifact in &first.receipt.artifacts {
        let file = broker.read(&artifact.path).unwrap();
        assert_eq!(file.revision, artifact.revision);
        assert_eq!(
            file.revision,
            Revision::compute(broker.vault_uuid(), &file.path, &file.content)
        );
        assert!(!file.content.is_empty());
    }
    workbench.tasks.project_pending().unwrap();
    let projected = workbench.tasks.broker().read(&page).unwrap();
    let text = String::from_utf8(projected.content).unwrap();
    assert!(text.contains("status: \"needs_review\""));
    assert!(text.contains(&format!(
        "last_attempt: \"{}\"",
        claimed.run_spec.attempt_id
    )));
    assert!(text.contains("custom: {keep: [1, 2]} # 作者备注"));
    assert!(text.ends_with("手写补充，不可被投影覆盖。\n"));
    assert!(
        workbench
            .tasks
            .control_db()
            .pending_outbox()
            .unwrap()
            .is_empty()
    );
    let events = workbench
        .tasks
        .audit_ledger()
        .query(&AuditFilter::default().with_task(task))
        .unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event().phase(), AuditPhase::Intent);
    assert_eq!(events[1].event().phase(), AuditPhase::Outcome);
    let mut index = InMemoryVaultIndex::new();
    index
        .rebuild(&[
            MarkdownDocument::new(page, text),
            MarkdownDocument::new(path("input.md"), "# 输入"),
        ])
        .unwrap();
    assert!(!index.backlinks(&path("input.md")).unwrap().is_empty());

    drop(workbench);
    let mut reopened =
        M0Workbench::open(&root.path().join("canonical"), &root.path().join("data")).unwrap();
    let reconciled = reopened.tasks.reconcile(20).unwrap();
    assert_eq!(reconciled.abandoned_attempts, 0);
    assert_eq!(reconciled.committed_attempts, 0);
    assert_eq!(
        reopened.tasks.task(task).unwrap().unwrap().status,
        TaskStatus::NeedsReview
    );
    assert_eq!(
        reopened
            .tasks
            .broker()
            .read(&first.receipt.manifest_path)
            .unwrap()
            .revision,
        first.receipt.manifest_revision
    );
}

#[test]
fn restart_abandons_old_running_attempt_and_does_not_requeue_it() {
    let (root, mut workbench) = fixture();
    let task = workbench.tasks.create_task(spec(), 1).unwrap();
    workbench.tasks.command(task, TaskCommand::Run, 2).unwrap();
    let claimed = workbench
        .tasks
        .claim(ClaimRequest {
            worker_id: "old-worker".into(),
            now_ms: 3,
            lease_until_ms: 100,
        })
        .unwrap()
        .unwrap();
    let started = workbench.runner.start(&claimed.run_spec).unwrap();
    workbench
        .tasks
        .attach_runner(
            task,
            claimed.run_spec.attempt_id,
            claimed.run_spec.fence,
            started.runner_ref,
            4,
        )
        .unwrap();
    let RunnerObservation::Completed(late_report) =
        workbench.runner.observe(&started.handle).unwrap()
    else {
        panic!("Stub 完成")
    };
    drop(workbench);
    let mut reopened =
        M0Workbench::open(&root.path().join("canonical"), &root.path().join("data")).unwrap();
    assert_eq!(reopened.tasks.reconcile(5).unwrap().abandoned_attempts, 1);
    assert_eq!(
        reopened.tasks.task(task).unwrap().unwrap().status,
        TaskStatus::Abandoned
    );
    assert!(
        reopened
            .tasks
            .report("late-report", late_report, 6)
            .is_err()
    );
    assert!(
        reopened
            .tasks
            .claim(ClaimRequest {
                worker_id: "new-worker".into(),
                now_ms: 7,
                lease_until_ms: 100
            })
            .unwrap()
            .is_none()
    );
    assert_eq!(reopened.tasks.reconcile(8).unwrap().abandoned_attempts, 0);
}

#[test]
fn handwritten_requested_action_or_status_cannot_start_a_task() {
    let (_root, mut workbench) = fixture();
    let task = workbench.tasks.create_task(spec(), 1).unwrap();
    workbench.tasks.project_pending().unwrap();
    let page = path(&format!("ops/tasks/{task}.md"));
    workbench = author_edit(workbench, &page, |text| {
        text.replace("requested_action: null", "requested_action: run")
            .replace("status: \"draft\"", "status: succeeded")
    });
    assert_eq!(
        workbench.tasks.task(task).unwrap().unwrap().status,
        TaskStatus::Draft
    );
    assert!(
        workbench
            .tasks
            .claim(ClaimRequest {
                worker_id: "worker".into(),
                now_ms: 3,
                lease_until_ms: 100
            })
            .unwrap()
            .is_none()
    );
}
