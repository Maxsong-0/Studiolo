use super::{
    BoardPlane, Engine,
    auth::{Grant, within},
    requests::*,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use studiolo_audit_ledger::AuditPhase;
use studiolo_task_engine::{TaskCommand, TaskEngine};

pub(super) fn parse<T: DeserializeOwned>(v: &Value) -> Result<T, String> {
    serde_json::from_value(v.clone()).map_err(|_| "请求字段缺失、多余或类型无效".into())
}
pub(super) fn value<T: Serialize>(v: T) -> Result<Value, String> {
    serde_json::to_value(v).map_err(|_| "响应编码失败".into())
}
fn task(engine: &Engine, r: &Request, id: studiolo_core::TaskId, hash: &str) -> Result<(), String> {
    let item = engine
        .board_detail(id, hash)
        .map_err(|e| e.to_string())?
        .ok_or("工作项不存在")?;
    if item.task.spec.project != r.project.as_str() {
        return Err("工作项不属于请求项目".into());
    }
    Ok(())
}
pub(super) fn owned(
    engine: &Engine,
    r: &Request,
    i: &Identity,
    g: &Grant,
    now: i64,
    allow_cancel: bool,
) -> Result<(), String> {
    task(engine, r, i.task_id, &g.hash)?;
    if allow_cancel {
        let a = engine
            .control_db()
            .attempt(i.task_id, i.attempt_id)
            .map_err(|e| e.to_string())?
            .ok_or("attempt不存在")?;
        if a.fence != i.fence || a.worker_id != r.instance_id {
            return Err("claim不属于当前实例".into());
        }
    } else if !engine
        .owns_board_claim(i.task_id, i.attempt_id, i.fence, &r.instance_id, now)
        .map_err(|e| e.to_string())?
    {
        return Err("claim已过期或不属于当前实例".into());
    }
    Ok(())
}
impl BoardPlane {
    pub(super) fn execute(&self, r: Request) -> Result<Value, String> {
        let id = format!("board-request:{}", ulid::Ulid::generate());
        // 排队结束后复核，避免等待锁期间的撤权仍能写入。
        let mut engine = self.engine.lock().map_err(|_| "黑板不可用")?;
        let grant = match self.authorize(&r) {
            Ok(g) => g,
            Err(reason) => {
                self.event(&r, &id, AuditPhase::Outcome, reason)?;
                return Err(reason.into());
            }
        };
        self.event(&r, &id, AuditPhase::Intent, "admitted")?;
        let now = jiff::Timestamp::now().as_millisecond();
        let outcome = self.dispatch(&mut engine, &r, &grant, now);
        // 状态机已提交时，投影失败不能把写入报告成未发生；调用方应刷新权威板。
        if outcome.is_ok()
            && !matches!(
                r.operation.as_str(),
                "list" | "get" | "batch.preview" | "memory.preview"
            )
            && self
                .project(&mut engine, r.project.as_str(), &grant.hash)
                .is_err()
        {
            self.event(&r, &id, AuditPhase::Outcome, "committed_projection_pending")?;
            return Err("权威状态已提交，但文件投影未完成；刷新黑板并修复投影后重试".into());
        }
        self.event(
            &r,
            &id,
            AuditPhase::Outcome,
            if outcome.is_ok() {
                "completed"
            } else {
                "rejected"
            },
        )?;
        self.authorize(&r)?;
        outcome
    }
    pub(super) fn project(
        &self,
        engine: &mut Engine,
        project: &str,
        hash: &str,
    ) -> Result<(), String> {
        let path = studiolo_core::VaultPath::parse(format!("{project}/04-协调/TASKS.md"))
            .map_err(|_| "项目投影路径无效")?;
        engine
            .project_board(&path, project, hash)
            .map_err(|_| "权威状态已提交，但文件投影未完成；刷新黑板并修复投影后重试".into())
    }
    fn dispatch(
        &self,
        engine: &mut Engine,
        r: &Request,
        g: &Grant,
        now: i64,
    ) -> Result<Value, String> {
        match r.operation.as_str() {
            "list" => {
                if r.request != json!({}) {
                    return Err("list不接受额外参数".into());
                }
                value(
                    engine
                        .board_list(r.project.as_str(), &g.hash)
                        .map_err(|e| e.to_string())?,
                )
            }
            "get" => {
                let a: Task = parse(&r.request)?;
                task(engine, r, a.task_id, &g.hash)?;
                value(
                    engine
                        .board_detail(a.task_id, &g.hash)
                        .map_err(|e| e.to_string())?,
                )
            }
            "create" => {
                let a: Create = parse(&r.request)?;
                if a.spec.task.project != r.project.as_str()
                    || a.spec
                        .task
                        .input_paths
                        .iter()
                        .chain(a.spec.files_owned.iter())
                        .any(|p| !within(p, &r.project))
                {
                    return Err("工作项项目或输入路径越界".into());
                }
                if let Some(parent) = a.spec.task.parent {
                    task(engine, r, parent, &g.hash)?;
                }
                let key = format!("{}:{}", r.instance_id, a.request_id);
                value(
                    engine
                        .create_board_task_by_instance(&key, a.spec, &g.hash, &r.instance_id, now)
                        .map_err(|e| e.to_string())?,
                )
            }
            "claim" => {
                let a: Claim = parse(&r.request)?;
                task(engine, r, a.task_id, &g.hash)?;
                if a.lease_until_ms > now.saturating_add(900_000) {
                    return Err("租约最多15分钟".into());
                }
                value(
                    engine
                        .claim_board_task(a.task_id, &r.instance_id, a.lease_until_ms, now)
                        .map_err(|e| e.to_string())?,
                )
            }
            "heartbeat" => {
                let a: Heartbeat = parse(&r.request)?;
                owned(engine, r, &a.identity, g, now, false)?;
                if a.lease_until_ms > now.saturating_add(900_000) {
                    return Err("租约最多15分钟".into());
                }
                engine
                    .heartbeat_progress(
                        a.identity.task_id,
                        a.identity.attempt_id,
                        a.identity.fence,
                        a.lease_until_ms,
                        a.tool_calls,
                        a.edits,
                        now,
                    )
                    .map_err(|e| e.to_string())?;
                Ok(json!({"ok":true}))
            }
            "handoff" => {
                let a: HandoffRequest = parse(&r.request)?;
                owned(engine, r, &a.identity, g, now, true)?;
                if a.handoff
                    .required_files
                    .iter()
                    .any(|f| !within(&f.path, &r.project))
                {
                    return Err("交接文件越界".into());
                }
                engine
                    .put_handoff(
                        a.identity.task_id,
                        a.identity.attempt_id,
                        a.identity.fence,
                        a.handoff,
                        now,
                    )
                    .map_err(|e| e.to_string())?;
                Ok(json!({"ok":true}))
            }
            "return" => {
                let a: Return = parse(&r.request)?;
                owned(engine, r, &a.identity, g, now, true)?;
                if a.receipt
                    .artifacts
                    .iter()
                    .chain(a.receipt.acceptance.iter().flat_map(|a| a.evidence.iter()))
                    .map(|p| &p.path)
                    .chain(a.receipt.unlanded.iter().map(|p| &p.path))
                    .chain(a.receipt.conflicts.iter())
                    .chain(a.receipt.closeout.destinations.iter())
                    .any(|p| !within(p, &r.project))
                {
                    return Err("产物路径越界".into());
                }
                value(
                    engine
                        .return_board_receipt(
                            a.identity.task_id,
                            a.identity.attempt_id,
                            a.identity.fence,
                            a.receipt,
                            now,
                        )
                        .map_err(|e| e.to_string())?,
                )
            }
            "cancel" => {
                let a: Identity = parse(&r.request)?;
                owned(engine, r, &a, g, now, true)?;
                let current = engine
                    .control_db()
                    .task(a.task_id)
                    .map_err(|e| e.to_string())?
                    .ok_or("工作项不存在")?;
                if current.active_attempt != Some(a.attempt_id) {
                    return Err("旧attempt不能取消后续claim".into());
                }
                value(
                    engine
                        .command(a.task_id, TaskCommand::Cancel, now)
                        .map_err(|e| e.to_string())?,
                )
            }
            "number" => {
                let a: Number = parse(&r.request)?;
                owned(engine, r, &a.identity, g, now, false)?;
                if !within(&a.scope, &r.project) {
                    return Err("编号范围越界".into());
                }
                value(
                    engine
                        .reserve_number(
                            &format!("{}:{}", r.instance_id, a.request_id),
                            a.identity.task_id,
                            &a.scope,
                            &a.prefix,
                        )
                        .map_err(|e| e.to_string())?,
                )
            }
            "batch.preview" | "batch.commit" | "batch.rollback" | "memory.preview" => {
                super::content::run(engine, r, g, now)
            }
            _ => Err("未知黑板操作".into()),
        }
    }
}
