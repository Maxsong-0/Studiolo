//! 路牌：按供给视图、策略版本与黑板实时状态派生冷启动路牌，超硬预算即审计并拒绝。
use super::{
    AgentSession,
    auditing::degraded,
    catalog::{HTTP_TOOLS, TOOLS},
    grant::Grant,
};
use rmcp::ErrorData as McpError;
use studiolo_supply::{
    HARD_BUDGET_TOKENS,
    audit::{self, Actor},
    signpost::{self, PolicyRule, PolicySignpost, Principal, SignpostInput},
};

impl AgentSession {
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
            // 名单外的规则母本既不提供准则，也不进第 6 项入口（ADR-026 D5）。
            rules: rules
                .into_iter()
                .filter(|record| grant.allows_rule(&record.meta.subject))
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
            // 路牌库文件兜底与读取路径同一名单；未接策略平面时不过滤（ADR-026 D5／D10）。
            rules_whitelist: grant
                .policy
                .as_ref()
                .map(|policy| policy.set.read.rules_whitelist.as_slice()),
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
