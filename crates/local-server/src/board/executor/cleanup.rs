//! Frozen-startup maintenance: only durable terminal attempts with confirmed isolation qualify.
use super::{RunnerConfig, landing_store::Lease, recovery::isolate_unit};
use crate::board::BoardPlane;
use std::{collections::BTreeSet, fs};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_control_db::{AttemptPhase, AttemptRecord};
use studiolo_runner_adapter::{RunnerRef, SnapshotCleanupRequest, cleanup_orphan_snapshots};

fn reference(attempt: &AttemptRecord, provider: &str) -> Option<RunnerRef> {
    if !matches!(
        attempt.phase,
        AttemptPhase::Completed
            | AttemptPhase::Failed
            | AttemptPhase::Cancelled
            | AttemptPhase::Abandoned
    ) {
        return None;
    }
    let r: RunnerRef = serde_json::from_value(attempt.runner_ref_json.clone()?).ok()?;
    (r.adapter == provider
        && r.pid.is_none()
        && r.start_time_ticks.is_none()
        && r.opaque_id
            == format!(
                "studiolo-runner-{}-{}.scope",
                attempt.attempt_id,
                attempt.fence.get()
            ))
    .then_some(r)
}
fn audit(
    board: &BoardPlane,
    id: &str,
    phase: AuditPhase,
    data: serde_json::Value,
) -> Result<(), String> {
    let event = AuditEvent::new(
        AuditEventId::parse(id).map_err(|_| "清理审计编号无效")?,
        phase,
        "runner.snapshot_cleanup",
        SanitizedPayload::try_from(data).map_err(|_| "清理审计无效")?,
    );
    board.audit.append(event).map_err(|_| "清理审计不可用")?;
    Ok(())
}
pub(super) fn startup(board: &BoardPlane, config: &RunnerConfig) -> Result<(), String> {
    // Called before socket bind, relay creation, admission and serving. Unknown objects remain
    // protected; age alone never authorizes deleting an attempt or terminating a process.
    let engine = board.engine.lock().map_err(|_| "清理期间黑板不可用")?;
    let control = engine.control_db();
    let boards = control.board_tasks().map_err(|_| "清理任务列表不可用")?;
    for profile in &config.providers {
        let entries = fs::read_dir(&profile.attempt_root)
            .map_err(|_| "执行目录枚举失败")?
            .take(4097)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "执行目录枚举失败")?;
        if entries.len() > 4096 {
            return Err("执行目录超过清理上限".into());
        }
        let all = entries.iter().map(|e| e.path()).collect::<BTreeSet<_>>();
        for project in &config.projects {
            let _lease = Lease::acquire(&config.state_root, &project.repository)?;
            let mut protected = all.clone();
            for task in &boards {
                let record = control
                    .task(task.task_id)
                    .map_err(|_| "清理任务不可用")?
                    .ok_or("清理任务缺失")?;
                if record
                    .definition_json
                    .get("project")
                    .and_then(serde_json::Value::as_str)
                    != Some(project.project.as_str())
                {
                    continue;
                }
                for attempt in control
                    .attempts_for_task(task.task_id)
                    .map_err(|_| "历史执行不可用")?
                {
                    let path = profile.attempt_root.join(format!(
                        "{}-{}-{}",
                        profile.provider.name(),
                        attempt.attempt_id,
                        attempt.fence.get()
                    ));
                    if !all.contains(&path) {
                        continue;
                    }
                    if let Some(r) = reference(&attempt, profile.provider.name()) {
                        isolate_unit(&r.opaque_id)?;
                        protected.remove(&path);
                    }
                }
            }
            let protected = protected.into_iter().collect::<Vec<_>>();
            let now = jiff::Timestamp::now();
            let id = format!(
                "snapshot-cleanup:{}:{}:{}",
                project.project,
                profile.provider.name(),
                now.as_nanosecond()
            );
            audit(
                board,
                &id,
                AuditPhase::Intent,
                serde_json::json!({"project":project.project,"provider":profile.provider.name(),"eligible":all.len()-protected.len()}),
            )?;
            let result = cleanup_orphan_snapshots(&SnapshotCleanupRequest {
                source_checkout: &project.repository,
                attempt_root: &profile.attempt_root,
                retained_root: &profile.retained_root,
                active_attempts: &protected,
                now_unix_seconds: u64::try_from(now.as_second()).map_err(|_| "主机时钟无效")?,
                minimum_age_seconds: 300,
            });
            let payload = match &result {
                Ok(report) => {
                    serde_json::json!({"removed":report.removed.len(),"retained":report.retained.len(),"protected":report.protected.len(),"blocked":report.blocked.len()})
                }
                Err(_) => serde_json::json!({"reason":"cleanup_failed"}),
            };
            audit(board, &id, AuditPhase::Outcome, payload)?;
            result.map_err(|_| "旧执行目录清理未确认")?;
        }
    }
    Ok(())
}
