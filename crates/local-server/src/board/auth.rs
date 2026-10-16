use super::{BoardPlane, requests::Request};
use studiolo_audit_ledger::{AuditEvent, AuditEventId, AuditPhase, AuditSubject, SanitizedPayload};
use studiolo_core::VaultPath;
use studiolo_policy::CapabilitySet;
use studiolo_supply::{Instance, SupplyError};

/// 身份库这一刻读不出来时的统一说法：结论未知，不是拒绝。
pub(super) const IDENTITY_UNAVAILABLE: &str = "身份库暂不可用，请稍后重试";

pub(super) struct Grant {
    pub hash: String,
    pub set: CapabilitySet,
}
/// 授权复核没有放行的两种结果；读不出来不能折成拒绝，调用方必须分开处理。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Refusal {
    /// 依据身份库、策略库里读到的数据确定拒绝。
    Denied(&'static str),
    /// 身份库、策略库或黑板控制存储这一刻读不出来；稍后重试可能通过。
    Unavailable(&'static str),
}
impl Refusal {
    pub(super) fn message(self) -> &'static str {
        match self {
            Self::Denied(message) | Self::Unavailable(message) => message,
        }
    }
}
/// 只报原因文本的调用方（人侧执行控制、执行体启动与落地）照旧拿到文本。
impl From<Refusal> for String {
    fn from(refusal: Refusal) -> Self {
        refusal.message().to_owned()
    }
}
/// 实例链三态：读到即有效；只有 `InvalidParent` 说明实例或父链确实无效，其余错误是读不出来。
pub(super) fn lineage(
    result: Result<Vec<Instance>, SupplyError>,
    invalid: &'static str,
) -> Result<Vec<Instance>, Refusal> {
    result.map_err(|error| match error {
        SupplyError::InvalidParent => Refusal::Denied(invalid),
        _ => Refusal::Unavailable(IDENTITY_UNAVAILABLE),
    })
}
pub(super) fn within(path: &VaultPath, root: &VaultPath) -> bool {
    path == root
        || path
            .as_str()
            .strip_prefix(root.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}
impl BoardPlane {
    pub(super) fn authorize(&self, r: &Request) -> Result<Grant, Refusal> {
        self.storage.check().map_err(Refusal::Unavailable)?;
        if !r.project.as_str().starts_with("项目/") || r.project.as_str().split('/').count() != 2
        {
            return Err(Refusal::Denied("必须选择完整项目路径"));
        }
        if !self
            .identities
            .type_is_active(&r.agent_type)
            .map_err(|_| Refusal::Unavailable(IDENTITY_UNAVAILABLE))?
        {
            return Err(Refusal::Denied("类型已失效"));
        }
        let chain = lineage(self.identities.instance_lineage(&r.instance_id), "实例无效")?;
        let mut result = None;
        for identity in chain {
            if identity.agent_type != r.agent_type {
                return Err(Refusal::Denied("实例类型不匹配"));
            }
            let a = self
                .policy
                .authorization(&r.agent_type, Some(&identity.id))
                .map_err(|_| Refusal::Unavailable("策略不可用"))?;
            if a.denial.is_some() {
                return Err(Refusal::Denied("实例或祖先已失效"));
            }
            let grant = a.grant.ok_or(Refusal::Denied("没有有效能力集"))?;
            if !grant
                .set
                .read
                .project_whitelist
                .iter()
                .filter_map(|p| VaultPath::parse(p).ok())
                .any(|p| within(&r.project, &p))
            {
                return Err(Refusal::Denied("项目不在白名单"));
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
                return Err(Refusal::Denied("未授予黑板写权限"));
            }
            if result.is_none() {
                result = Some(Grant {
                    hash: a.effective_hash,
                    set: grant.set,
                });
            }
        }
        result.ok_or(Refusal::Denied("身份无效"))
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

#[cfg(test)]
mod tests {
    use super::{IDENTITY_UNAVAILABLE, Refusal, lineage};
    use studiolo_supply::{Instance, SupplyError};

    /// 人侧落地复核原执行身份也走这里，所以单独钉住分类本身，不只靠黑板入口的契约测试。
    #[test]
    fn only_an_invalid_parent_denies_and_every_other_store_error_is_unavailable() {
        assert_eq!(
            lineage(Err(SupplyError::InvalidParent), "原执行身份失效"),
            Err(Refusal::Denied("原执行身份失效"))
        );
        for error in [
            SupplyError::StoreUnavailable,
            SupplyError::UnsafeStorage,
            SupplyError::CorruptStore,
            SupplyError::MissingStore,
        ] {
            assert_eq!(
                lineage(Err(error), "原执行身份失效"),
                Err(Refusal::Unavailable(IDENTITY_UNAVAILABLE))
            );
        }
        let chain = vec![Instance {
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".into(),
            agent_type: "claude".into(),
            registered_at: "2026-09-20T00:00:00Z".into(),
        }];
        assert_eq!(lineage(Ok(chain.clone()), "原执行身份失效"), Ok(chain));
        assert_eq!(
            String::from(Refusal::Unavailable(IDENTITY_UNAVAILABLE)),
            IDENTITY_UNAVAILABLE
        );
    }
}
