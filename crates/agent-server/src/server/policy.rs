//! 策略工具族：policy_read 读策略平面 effective，propose_desired 只把提案入库为 pending。

use super::{AgentSession, PolicyReadArgs, ProposeArgs, auditing};
use rmcp::ErrorData as McpError;
use studiolo_policy::{PolicyKind, ProposalInput};

impl AgentSession {
    /// 读策略平面 effective：规则母本与 skills 是「读取即外发」的内容，同样先审计后返回。
    pub(crate) async fn text_policy_read(&self, args: PolicyReadArgs) -> Result<String, McpError> {
        let grant = self.authorize("policy_read")?;
        let actor = grant.actor();
        let Some(store) = &self.state.policy else {
            self.deny(actor, "policy_read", "policy_plane_absent");
            return Err(McpError::invalid_request(
                "策略平面未接入（M3 兼容模式）",
                None,
            ));
        };
        let unavailable = |_| McpError::internal_error("策略库不可用，本次拒绝供给", None);
        let kind = match args.kind.as_str() {
            "rules" => PolicyKind::Rules,
            "skills" => PolicyKind::Skills,
            _ => {
                return Err(McpError::invalid_params(
                    "kind 只能是 rules 或 skills",
                    None,
                ));
            }
        };
        // 版本行与条目取自同一读事务：人侧进程在两次读取之间发布，也不会让二者各属一版。
        let header = |hash: &str| format!("# 策略版本 {}\n", &hash[..12]);
        match (kind, args.subject) {
            (PolicyKind::Rules, None) => {
                let rules = store.rules().map_err(unavailable)?;
                self.audit_read(&grant.instance, "policy://rules", "-", "policy_read")?;
                let lines = rules
                    .value
                    .iter()
                    .map(|record| {
                        format!(
                            "{}\tv{}\t{}",
                            record.meta.subject,
                            record.meta.version,
                            &record.meta.body_sha256[..12]
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(format!(
                    "{}{} 份规则母本\n{}",
                    header(&rules.effective_hash),
                    lines.len(),
                    lines.join("\n")
                ))
            }
            (PolicyKind::Rules, Some(subject)) => {
                let rules = store.rules().map_err(unavailable)?;
                let header = header(&rules.effective_hash);
                let Some(record) = rules
                    .value
                    .into_iter()
                    .find(|record| record.meta.subject == subject)
                else {
                    self.deny(actor, "policy_read", "not_found");
                    return Err(McpError::resource_not_found("规则母本不存在", None));
                };
                let path = format!("policy://rules/{subject}");
                let revision = format!("v{}", record.meta.version);
                self.audit_read(&grant.instance, &path, &revision, "policy_read")?;
                Ok(format!(
                    "{header}{path}@{revision} sha256 {}\n{}",
                    record.meta.body_sha256,
                    record.body.unwrap_or_default()
                ))
            }
            (PolicyKind::Skills, None) => {
                let skills = store.skills().map_err(unavailable)?;
                self.audit_read(&grant.instance, "policy://skills", "-", "policy_read")?;
                let lines = skills
                    .value
                    .iter()
                    .map(|(skill, meta)| {
                        format!(
                            "{}\tv{}\t{}\t{}",
                            skill.name,
                            meta.version,
                            skill.summary,
                            skill.triggers.join(" | ")
                        )
                    })
                    .collect::<Vec<_>>();
                Ok(format!(
                    "{}{} 个 skill（索引式候选；硬绑定见路牌第 5 项）\n{}",
                    header(&skills.effective_hash),
                    lines.len(),
                    lines.join("\n")
                ))
            }
            (PolicyKind::Skills, Some(name)) => {
                let skills = store.skills().map_err(unavailable)?;
                let header = header(&skills.effective_hash);
                let Some((skill, meta)) = skills
                    .value
                    .into_iter()
                    .find(|(skill, _)| skill.name == name)
                else {
                    self.deny(actor, "policy_read", "not_found");
                    return Err(McpError::resource_not_found("skill 不存在", None));
                };
                let path = format!("policy://skills/{name}");
                let revision = format!("v{}", meta.version);
                self.audit_read(&grant.instance, &path, &revision, "policy_read")?;
                Ok(format!(
                    "{header}{path}@{revision} sha256 {}\n摘要：{}\n触发：{}\n\n{}",
                    meta.body_sha256,
                    skill.summary,
                    skill.triggers.join(" | "),
                    skill.markdown
                ))
            }
            _ => Err(McpError::invalid_params(
                "kind 只能是 rules 或 skills",
                None,
            )),
        }
    }

    /// 提交 desired 提案：只入库为 `pending`，永不自动生效；须能力集 `policy.propose_desired`。
    pub(crate) async fn text_propose(&self, args: ProposeArgs) -> Result<String, McpError> {
        let grant = self.authorize("propose_desired")?;
        let actor = grant.actor();
        let (Some(store), Some(policy)) = (&self.state.policy, &grant.policy) else {
            self.deny(actor, "propose_desired", "policy_plane_absent");
            return Err(McpError::invalid_request(
                "策略平面未接入（M3 兼容模式）",
                None,
            ));
        };
        if !policy.set.policy.propose_desired {
            self.deny(actor, "propose_desired", "capability_denied");
            return Err(McpError::invalid_request(
                "能力集未授予提交 desired 提案的能力",
                None,
            ));
        }
        let kind = PolicyKind::parse(&args.kind)
            .map_err(|_| McpError::invalid_params("kind 无效", None))?;
        let record = store
            .propose(
                &grant.instance.agent_type,
                &grant.instance.id,
                ProposalInput {
                    kind,
                    subject: args.subject,
                    body: args.body,
                },
                &crate::state::now(),
            )
            .map_err(|error| match error {
                studiolo_policy::PolicyError::TooManyProposals => {
                    McpError::invalid_request("待处理提案已达上限，请等待主人处理", None)
                }
                studiolo_policy::PolicyError::InvalidSubject
                | studiolo_policy::PolicyError::InvalidKind
                | studiolo_policy::PolicyError::InvalidBody(_) => {
                    McpError::invalid_params(error.to_string(), None)
                }
                _ => McpError::internal_error("策略库不可用，本次拒绝", None),
            })?;
        let event = studiolo_policy::audit::desired_submitted(
            studiolo_policy::audit::Principal::Agent {
                agent_type: &grant.instance.agent_type,
                instance_id: &grant.instance.id,
            },
            &record,
        )
        .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        self.state
            .audit(event)
            .map_err(|outage| auditing::degraded(&outage))?;
        let report = record.report.as_ref();
        let problems = report
            .map(|report| {
                report
                    .problems
                    .iter()
                    .map(|problem| format!("- {}：{}", problem.path, problem.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        Ok(format!(
            "desired_id {}\nstatus {}\nok {}\nexpansions {}\n提案永不自动生效；须主人在人侧经 One-time Code 发布。\n{problems}",
            record.id,
            record.status.as_str(),
            report.is_some_and(|report| report.ok),
            report.map_or(0, |report| report.expansions.len()),
        ))
    }
}
