//! 策略工具族：policy_read 读策略平面 effective，propose_desired 只把提案入库为 pending。

use super::{AgentSession, PolicyReadArgs, ProposeArgs, auditing};
use rmcp::ErrorData as McpError;
use studiolo_policy::{PolicyKind, ProposalInput, sha256_hex};

/// 清单列里的控制字符与 Unicode 行／段分隔符压成空格，保证清单一行一项。
fn one_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
                ' '
            } else {
                character
            }
        })
        .collect()
}

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
        // 规则名单取自授权复核时的能力集，规则条目还须与它同属一个 effective；否则人侧恰在两次读取
        // 之间发布了新版本，名单与条目会各属一版，失败关闭让调用方重试（与路牌同一做法）。
        let same_effective = |hash: &str| match &grant.policy {
            Some(policy) if policy.effective_hash != hash => Err(McpError::internal_error(
                "策略在本次调用途中发布了新版本，本次拒绝供给，请重试",
                None,
            )),
            _ => Ok(()),
        };
        match (kind, args.subject) {
            (PolicyKind::Rules, None) => {
                let rules = store.rules().map_err(unavailable)?;
                same_effective(&rules.effective_hash)?;
                self.audit_read(&grant.instance, "policy://rules", "-", "policy_read")?;
                // 名单外的规则母本不进清单，行数与计数一致（ADR-026 D5）。
                let lines = rules
                    .value
                    .iter()
                    .filter(|record| grant.allows_rule(&record.meta.subject))
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
                // 名单外与不存在对外同一句：接入方无从分辨名单外的规则是否存在（ADR-026 D5）。
                if !grant.allows_rule(&subject) {
                    self.deny(actor, "policy_read", "rule_not_whitelisted");
                    return Err(McpError::resource_not_found("规则母本不存在", None));
                }
                let rules = store.rules().map_err(unavailable)?;
                same_effective(&rules.effective_hash)?;
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
                // 一个 skill 恰占一行：摘要与触发里的换行等控制字符压成空格，否则一条 skill 会拆成
                // 多行、与计数不符；原文以单条读取的规范 JSON 为准。
                let lines = skills
                    .value
                    .iter()
                    .map(|(skill, meta)| {
                        format!(
                            "{}\tv{}\t{}\t{}",
                            skill.name,
                            meta.version,
                            one_line(&skill.summary),
                            skill
                                .triggers
                                .iter()
                                .map(String::as_str)
                                .map(one_line)
                                .collect::<Vec<_>>()
                                .join(" | ")
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
                let Some((_, meta)) = skills
                    .value
                    .into_iter()
                    .find(|(skill, _)| skill.name == name)
                else {
                    self.deny(actor, "policy_read", "not_found");
                    return Err(McpError::resource_not_found("skill 不存在", None));
                };
                // 正文是入库的规范 JSON 原文：版本不可变，按版本号取回的就是入库字节，sha256 行即其摘要，
                // 读方直接按收到的字节核对。取不回或摘要对不上即失败关闭，不拿重新序列化的文本顶替。
                let body = store
                    .version(meta.version)
                    .map_err(unavailable)?
                    .and_then(|record| record.body)
                    .filter(|body| sha256_hex(body.as_bytes()) == meta.body_sha256)
                    .ok_or_else(|| McpError::internal_error("策略库返回不一致", None))?;
                let path = format!("policy://skills/{name}");
                let revision = format!("v{}", meta.version);
                self.audit_read(&grant.instance, &path, &revision, "policy_read")?;
                Ok(format!(
                    "{header}{path}@{revision} sha256 {}\n{body}",
                    meta.body_sha256
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
