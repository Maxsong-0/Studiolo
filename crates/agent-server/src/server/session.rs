//! 会话认证、策略平面复核（三级吊销与能力集）、先审计后供给，及带新鲜度与策略版本的预算路牌。
use super::{AgentSession, HTTP_TOOLS, TOOLS};
use crate::state::{AppState, AuditUnavailable, Generation, now, now_ms};
use rmcp::ErrorData as McpError;
use std::sync::{Arc, OnceLock};
use studiolo_core::VaultPath;
use studiolo_policy::{CapabilitySet, Denial, view::intersect_prefixes};
use studiolo_supply::{
    HARD_BUDGET_TOKENS, Instance, ReadRecord,
    audit::{self, Actor},
    signpost::{self, PolicyRule, PolicySignpost, Principal, SignpostInput},
};

/// 策略平面拒绝（吊销／零权限）的固定前缀：握手时像降级握手一样「应答但一项不给」，
/// 外壳据此把它当授权拒绝而不是会话失效。
pub(crate) const POLICY_DENIED_PREFIX: &str = "策略拒绝：";

pub(super) fn degraded(outage: &AuditUnavailable) -> McpError {
    McpError::internal_error(
        format!(
            "审计不可用（outage {}）：本次读写全部拒绝",
            outage.outage_id
        ),
        None,
    )
}

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
    /// 新会话：实例在 `initialize` 时铸造。
    pub fn new(state: Arc<AppState>) -> Self {
        Self {
            state,
            session_ref: ulid::Ulid::generate().to_string(),
            instance: OnceLock::new(),
            http: false,
            mcp: None,
        }
    }

    /// MCP 外壳在预占后构造，绑定实际传输会话的身份与期限。
    pub(crate) fn new_mcp(
        state: Arc<AppState>,
        binding: Arc<crate::mcp_transport::Binding>,
    ) -> Self {
        Self {
            mcp: Some(binding),
            ..Self::new(state)
        }
    }

    pub(super) fn finish_mcp(&self, text: String) -> Result<String, McpError> {
        if let Some(binding) = &self.mcp
            && (!binding.current(&self.state.store) || !binding.touch(std::time::Instant::now()))
        {
            return Err(McpError::invalid_request("会话无效，请重新握手", None));
        }
        Ok(text)
    }

    /// HTTP 会话共用供给逻辑，只替换预算内的续读入口。
    pub(crate) fn new_http(state: Arc<AppState>) -> Self {
        Self {
            http: true,
            ..Self::new(state)
        }
    }

    /// 会话登记后绑定的类型，供 HTTP 外壳匹配每次请求的认证身份。
    pub(crate) fn bound_type(&self) -> Option<&str> {
        self.instance
            .get()
            .map(|instance| instance.agent_type.as_str())
    }

    /// 已登记的实例；外壳可回传身份头，不包含会话句柄或类型令牌。
    pub(crate) fn bound_instance(&self) -> Option<&Instance> {
        self.instance.get()
    }

    /// 由外壳在新会话发布前调用一次；注册、策略复核和路牌均先审计后交付。
    pub(crate) async fn initialize_text(&self, agent_type: &str) -> Result<String, McpError> {
        self.initialize_with_parent(agent_type, None).await
    }

    pub(crate) async fn initialize_with_parent(
        &self,
        agent_type: &str,
        parent: Option<&str>,
    ) -> Result<String, McpError> {
        let instance = if let Some(parent) = parent {
            if self.state.policy.is_none() {
                return Err(McpError::invalid_request("子实例需要有效策略平面", None));
            }
            let ancestors = self
                .state
                .store
                .instance_lineage(parent)
                .map_err(|_| McpError::invalid_params("父实例无效", None))?;
            let parent_instance = ancestors
                .first()
                .filter(|instance| instance.agent_type == agent_type)
                .ok_or_else(|| McpError::invalid_params("父实例类型不匹配", None))?;
            self.grant_for(parent_instance, "instance.register_child")?;
            self.state
                .store
                .register_child_instance(agent_type, parent, &self.session_ref, &now())
        } else {
            self.state
                .store
                .register_instance(agent_type, &self.session_ref, &now())
        }
        .map_err(|_| McpError::internal_error("实例注册失败", None))?;
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let registered = audit::instance_registered(actor)
            .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        let instructions = match self.state.audit(registered) {
            Err(outage) => Err(McpError::internal_error(
                format!(
                    "审计不可用：outage {}。本次会话所有读取与写入全部拒绝，不提供路牌；请稍后重试。",
                    outage.outage_id
                ),
                None,
            )),
            Ok(_) => match self.grant_for(&instance, "handshake") {
                Ok(grant) => self.signpost(&grant).await,
                Err(error) => Err(error),
            },
        };
        // MCP 仍保留降级实例与被策略拒绝的实例，使初始化返回原有无内容 instructions
        // （后续每次调用都会再次复核并再次拒绝）；HTTP 分别映射为 503 与 403。
        if instructions.is_ok()
            || instructions.as_ref().is_err_and(|error| {
                error.message.contains("审计不可用")
                    || error.message.starts_with(POLICY_DENIED_PREFIX)
            })
        {
            let _ = self.instance.set(instance);
        }
        instructions
    }

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

    pub(super) fn deny(&self, actor: Actor<'_>, action: &str, reason: &str) {
        if let Ok(event) = audit::action_denied(actor, action, reason) {
            let _ = self.state.audit(event);
        }
    }

    /// 内容供给前的审计：失败即拒绝返回。
    pub(super) fn audit_read(
        &self,
        instance: &Instance,
        path: &str,
        revision: &str,
        tool: &str,
    ) -> Result<(), McpError> {
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let event = audit::content_read(actor, path, revision, tool)
            .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        let ack = self
            .state
            .audit(event)
            .map_err(|outage| degraded(&outage))?;
        let _ = self.state.store.record_read(&ReadRecord {
            recorded_at_ms: ack.recorded_at_ms,
            recorded_at: ack.recorded_at,
            agent_type: instance.agent_type.clone(),
            instance_id: instance.id.clone(),
            path: path.to_owned(),
            revision: revision.to_owned(),
            tool: tool.to_owned(),
            ledger_offset: ack.offset,
        });
        Ok(())
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

    /// 派生路牌并审计；返回文本。
    pub(super) async fn generation(
        &self,
        instance: &Instance,
        action: &str,
    ) -> Result<Arc<Generation>, McpError> {
        match self.state.verified().await {
            Ok(generation) => Ok(generation),
            Err(reason) => {
                let event = audit::action_denied(
                    Actor::instance(&instance.agent_type, &instance.id),
                    action,
                    "replica_unavailable",
                )
                .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                self.state
                    .audit(event)
                    .map_err(|outage| degraded(&outage))?;
                Err(McpError::internal_error(
                    format!("副本不可用：{reason}；本次拒绝供给内容"),
                    None,
                ))
            }
        }
    }

    pub(super) fn freshness(generation: &Generation) -> Result<String, McpError> {
        generation
            .freshness
            .as_ref()
            .map_or(Ok(String::new()), |freshness| {
                freshness
                    .describe(now_ms())
                    .map(|text| format!("{text}\n"))
                    .map_err(|error| McpError::internal_error(error.to_string(), None))
            })
    }

    fn policy_signpost(&self, grant: &Grant) -> Result<Option<PolicySignpost>, McpError> {
        let (Some(policy), Some(store)) = (&grant.policy, &self.state.policy) else {
            return Ok(None);
        };
        let unavailable = |_| McpError::internal_error("策略库不可用，本次拒绝供给", None);
        // 规则与 skills 同属一个读事务；它的 effective 哈希还须等于本次授权复核时的哈希，
        // 否则人侧恰在两次读取之间发布了新版本，能力集与条目会各属一版，失败关闭让调用方重试。
        let current = store.rules_and_skills().map_err(unavailable)?;
        if current.effective_hash != policy.effective_hash {
            return Err(McpError::internal_error(
                "策略在本次调用途中发布了新版本，本次拒绝供给，请重试",
                None,
            ));
        }
        let studiolo_policy::CurrentEntries { rules, skills } = current.value;
        Ok(Some(PolicySignpost {
            board_write: !self.http && self.state.board.is_some() && policy.set.write.board,
            effective_hash: policy.effective_hash.clone(),
            capability_version: policy.version,
            read_prefixes: grant.allowed.clone(),
            content_patch_only: policy.set.write.content_patch_only,
            propose_desired: policy.set.policy.propose_desired,
            hard_bindings: policy
                .set
                .hard_bound_skills
                .iter()
                .map(|binding| (binding.when.clone(), binding.must_use.clone()))
                .collect(),
            rules: rules
                .into_iter()
                .map(|record| PolicyRule {
                    subject: record.meta.subject,
                    version: record.meta.version,
                    body: record.body.unwrap_or_default(),
                })
                .collect(),
            skills: skills.into_iter().map(|(skill, _)| skill.name).collect(),
        }))
    }

    pub(super) async fn signpost(&self, grant: &Grant) -> Result<String, McpError> {
        let instance = &grant.instance;
        let generation = self.generation(instance, "handshake").await?;
        let freshness = Self::freshness(&generation)?;
        let policy = self.policy_signpost(grant)?;
        let principal = Principal {
            agent_type: instance.agent_type.clone(),
            instance_id: instance.id.clone(),
            registered_at: instance.registered_at.clone(),
        };
        let actor = Actor::instance(&instance.agent_type, &instance.id);
        let input = SignpostInput {
            snapshot: &generation.snapshot,
            allow: &grant.allowed,
            principal: &principal,
            generation: generation.number,
            generated_at: &generation.generated_at,
            freshness: (!freshness.is_empty()).then_some(freshness.as_str()),
            tools: if self.http { HTTP_TOOLS } else { TOOLS },
            policy: policy.as_ref(),
        };
        let derived = if self.http {
            signpost::derive_http(&input)
        } else {
            signpost::derive(&input)
        };
        match derived {
            Ok(mut signpost) => {
                let board = self.board_signpost(grant).await?;
                let at = signpost
                    .text
                    .find("## 5 硬约束")
                    .ok_or_else(|| McpError::internal_error("路牌格式无效", None))?;
                signpost.text.insert_str(at, &board);
                signpost.tokens = studiolo_supply::count_tokens(&signpost.text);
                if signpost.tokens > HARD_BUDGET_TOKENS {
                    let event = audit::handshake_budget_exceeded(
                        actor,
                        signpost.tokens,
                        HARD_BUDGET_TOKENS,
                    )
                    .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                    self.state
                        .audit(event)
                        .map_err(|outage| degraded(&outage))?;
                    return Err(McpError::internal_error(
                        "路牌超过2000 token硬预算，拒绝供给",
                        None,
                    ));
                }
                let event = audit::handshake_served(
                    actor,
                    generation.number,
                    signpost.tokens,
                    policy.as_ref().map(|policy| policy.effective_hash.as_str()),
                )
                .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                self.state
                    .audit(event)
                    .map_err(|outage| degraded(&outage))?;
                Ok(signpost.text)
            }
            Err(exceeded) => {
                let event =
                    audit::handshake_budget_exceeded(actor, exceeded.tokens, exceeded.limit)
                        .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
                self.state
                    .audit(event)
                    .map_err(|outage| degraded(&outage))?;
                Err(McpError::internal_error(
                    format!(
                        "路牌 {} token 超过硬预算 {}，握手失败（二元验收）",
                        exceeded.tokens, HARD_BUDGET_TOKENS
                    ),
                    None,
                ))
            }
        }
    }
}
