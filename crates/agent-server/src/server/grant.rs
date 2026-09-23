//! 授权复核：每次调用重验会话、实例与类型，再按策略平面三级吊销与能力集求出本次供给视图。
use super::{AgentSession, session::POLICY_DENIED_PREFIX};
use crate::state::now;
use rmcp::ErrorData as McpError;
use studiolo_core::VaultPath;
use studiolo_policy::{CapabilitySet, Denial, view::intersect_prefixes};
use studiolo_supply::{Instance, audit::Actor};

/// 策略平面复核通过后拿到的能力集（每次调用重新读取，不缓存）。
pub(crate) struct GrantPolicy {
    /// 复核时的 effective 哈希。
    pub effective_hash: String,
    /// 能力集版本。
    pub version: u64,
    /// 能力集本身。
    pub set: CapabilitySet,
}

/// 一次调用的授权结果：实例、供给视图与（接入策略平面时的）能力集。
pub(crate) struct Grant {
    /// 已登记实例。
    pub instance: Instance,
    /// 供给视图：部署上限 ∩ 能力集白名单；M3 兼容模式下就是部署上限。
    pub allowed: Vec<VaultPath>,
    /// 策略平面派生项；`None` ＝ 未接入。
    pub policy: Option<GrantPolicy>,
}

impl Grant {
    /// 路径是否在供给视图内。
    pub(crate) fn allows(&self, path: &VaultPath) -> bool {
        self.allowed
            .iter()
            .any(|prefix| path == prefix || path.as_str().starts_with(&format!("{prefix}/")))
    }

    /// 审计主体。
    pub(crate) fn actor(&self) -> Actor<'_> {
        Actor::instance(&self.instance.agent_type, &self.instance.id)
    }
}

impl AgentSession {
    /// 每次调用都重验：实例已注册、类型未吊销（身份库）、策略平面三级吊销与能力集（策略库）。
    pub(super) fn authorize(&self, action: &str) -> Result<Grant, McpError> {
        if self.mcp.as_ref().is_some_and(|binding| {
            !binding.live(std::time::Instant::now()) || !binding.current(&self.state.store)
        }) {
            return Err(McpError::invalid_request("会话无效，请重新握手", None));
        }
        let Some(instance) = self.instance.get() else {
            return Err(McpError::invalid_request("会话尚未完成 initialize", None));
        };
        let active = self
            .state
            .store
            .type_is_active(&instance.agent_type)
            .map_err(|_| McpError::internal_error("身份库不可用", None))?;
        if !active {
            self.deny(
                Actor::instance(&instance.agent_type, &instance.id),
                action,
                "type_revoked",
            );
            return Err(McpError::invalid_request("类型已吊销", None));
        }
        let grant = self.grant_for(instance, action)?;
        let _ = self.state.store.touch_instance(&instance.id, &now());
        Ok(grant)
    }

    /// 策略平面复核：全局吊销 → 类型吊销 → 实例吊销 → 能力集；被拒即审计并失败关闭。
    ///
    /// 未接入策略平面时（M3 兼容模式）供给视图就是部署上限。
    pub(super) fn grant_for(&self, instance: &Instance, action: &str) -> Result<Grant, McpError> {
        let Some(policy) = &self.state.policy else {
            return Ok(Grant {
                instance: instance.clone(),
                allowed: self.state.allow.clone(),
                policy: None,
            });
        };
        let lineage = self
            .state
            .store
            .instance_lineage(&instance.id)
            .map_err(|_| McpError::internal_error("实例关系不可用，本次拒绝供给", None))?;
        for ancestor in lineage.iter().skip(1) {
            if ancestor.agent_type != instance.agent_type {
                return Err(McpError::invalid_request("实例关系无效", None));
            }
            let check = policy
                .authorization(&ancestor.agent_type, Some(&ancestor.id))
                .map_err(|_| McpError::internal_error("策略库不可用，本次拒绝供给", None))?;
            if check.denial.is_some() || check.grant.is_none() {
                self.deny(
                    Actor::instance(&instance.agent_type, &instance.id),
                    action,
                    "ancestor_revoked",
                );
                return Err(McpError::invalid_request(
                    format!("{POLICY_DENIED_PREFIX}父实例已失效"),
                    None,
                ));
            }
        }
        let authorization = policy
            .authorization(&instance.agent_type, Some(&instance.id))
            .map_err(|_| McpError::internal_error("策略库不可用，本次拒绝供给", None))?;
        if let Some(denial) = authorization.denial {
            self.deny(
                Actor::instance(&instance.agent_type, &instance.id),
                action,
                denial.as_str(),
            );
            return Err(McpError::invalid_request(
                format!(
                    "{POLICY_DENIED_PREFIX}{}；本次会话所有读取与写入全部拒绝",
                    match denial {
                        Denial::GlobalRevoked => "全局吊销生效中",
                        Denial::TypeRevoked => "类型已被吊销",
                        Denial::InstanceRevoked => "实例已被吊销",
                        Denial::NoCapabilitySet => "该类型没有生效的能力集（零权限）",
                    }
                ),
                None,
            ));
        }
        let grant = authorization
            .grant
            .ok_or_else(|| McpError::internal_error("策略库返回不一致", None))?;
        let whitelist = grant
            .set
            .read
            .project_whitelist
            .iter()
            .filter_map(|prefix| VaultPath::parse(prefix).ok())
            .collect::<Vec<_>>();
        let allowed = intersect_prefixes(&self.state.allow, &whitelist);
        Ok(Grant {
            instance: instance.clone(),
            allowed,
            policy: Some(GrantPolicy {
                effective_hash: authorization.effective_hash,
                version: grant.meta.version,
                set: grant.set,
            }),
        })
    }

    pub(super) fn allowed_project(
        &self,
        grant: &Grant,
        action: &str,
        project: &str,
    ) -> Result<VaultPath, McpError> {
        match grant
            .allowed
            .iter()
            .find(|prefix| prefix.as_str() == project)
        {
            Some(prefix) => Ok(prefix.clone()),
            None => {
                self.deny(grant.actor(), action, "outside_allowlist");
                Err(McpError::invalid_params("项目不在供给白名单内", None))
            }
        }
    }
}
