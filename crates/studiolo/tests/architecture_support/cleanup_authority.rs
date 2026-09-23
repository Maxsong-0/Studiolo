//! M6 最终实现的精确调用清单与边界锚点。
use std::path::Path;
pub(super) fn remainder(crate_name: &str, path: &Path, source: &str) -> String {
    if crate_name != "studiolo-local-server" {
        return source.to_owned();
    }
    if path.ends_with("crates/local-server/src/board/executor/cleanup.rs") {
        return super::authority_tokens::checked_closed(
            source,
            &[
                "letcontrol=engine.control_db();",
                "letmutprotected=all.clone();",
                "ifentries.len()>4096",
                ".take(4097)",
                ".attempts_for_task(task.task_id)",
                "let_lease=Lease::acquire(&config.state_root,&project.repository)?;",
                "isolate_unit(&r.opaque_id)?;protected.remove(&path);",
                "active_attempts:&protected",
                "AttemptPhase::Completed|AttemptPhase::Failed|AttemptPhase::Cancelled|AttemptPhase::Abandoned",
                "r.adapter==provider&&r.pid.is_none()&&r.start_time_ticks.is_none()",
                "\"studiolo-runner-{}-{}.scope\",attempt.attempt_id,attempt.fence.get()",
                "AuditPhase::Intent",
                "AuditPhase::Outcome",
            ],
            &[("fs::read_dir(&profile.attempt_root)", 1)],
        );
    }
    source.to_owned()
}
