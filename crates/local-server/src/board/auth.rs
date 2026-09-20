use super::{BoardPlane, requests::Request};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, AuditSubject, SanitizedPayload};
use studiolo_core::VaultPath;
use studiolo_policy::CapabilitySet;

pub(super) struct Grant {
    pub hash: String,
    pub set: CapabilitySet,
}
pub(super) fn within(path: &VaultPath, root: &VaultPath) -> bool {
    path == root
        || path
            .as_str()
            .strip_prefix(root.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}
impl BoardPlane {
    pub(super) fn authorize(&self, r: &Request) -> Result<Grant, &'static str> {
        self.storage.check()?;
        if !r.project.as_str().starts_with("项目/") || r.project.as_str().split('/').count() != 2
        {
            return Err("必须选择完整项目路径");
        }
        if !self
            .identities
            .type_is_active(&r.agent_type)
            .map_err(|_| "身份不可用")?
        {
            return Err("类型已失效");
        }
        let lineage = self
            .identities
            .instance_lineage(&r.instance_id)
            .map_err(|_| "实例无效")?;
        let mut result = None;
        for identity in lineage {
            if identity.agent_type != r.agent_type {
                return Err("实例类型不匹配");
            }
            let a = self
                .policy
                .authorization(&r.agent_type, Some(&identity.id))
                .map_err(|_| "策略不可用")?;
            if a.denial.is_some() {
                return Err("实例或祖先已失效");
            }
            let grant = a.grant.ok_or("没有有效能力集")?;
            if !grant
                .set
                .read
                .project_whitelist
                .iter()
                .filter_map(|p| VaultPath::parse(p).ok())
                .any(|p| within(&r.project, &p))
            {
                return Err("项目不在白名单");
            }
            if !matches!(r.operation.as_str(), "list" | "get")
                && (!grant.set.write.board
                    || !grant
                        .set
                        .write
                        .board_projects
                        .iter()
                        .any(|p| p == r.project.as_str()))
            {
                return Err("未授予黑板写权限");
            }
            if result.is_none() {
                result = Some(Grant {
                    hash: a.effective_hash,
                    set: grant.set,
                });
            }
        }
        result.ok_or("身份无效")
    }
    pub(super) fn event(
        &self,
        r: &Request,
        id: &str,
        phase: AuditPhase,
        reason: &str,
    ) -> Result<(), &'static str> {
        let payload=SanitizedPayload::try_from(serde_json::json!({"agent_type":r.agent_type,"agent_instance":r.instance_id,"project":r.project,"operation":r.operation,"reason":reason})).map_err(|_|"审计编码失败")?;
        let mut subject = AuditSubject::default();
        if let Some(id) = r
            .request
            .get("task_id")
            .or_else(|| r.request.pointer("/identity/task_id"))
            && let Ok(task) = serde_json::from_value(id.clone())
        {
            subject = subject.with_task(task);
        }
        if let Some(id) = r
            .request
            .get("attempt_id")
            .or_else(|| r.request.pointer("/identity/attempt_id"))
            && let Ok(attempt) = serde_json::from_value(id.clone())
        {
            subject = subject.with_attempt(attempt);
        }
        let event = AuditEvent::new(
            AuditEventId::parse(id).map_err(|_| "审计编号失败")?,
            phase,
            "board.request",
            payload,
        )
        .with_subject(subject);
        self.audit.append(event).map_err(|_| "审计不可用")?;
        Ok(())
    }
}
