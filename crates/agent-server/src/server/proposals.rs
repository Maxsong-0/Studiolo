//! 提案工具族：propose_desired 把提案入库为 pending（断线重提拿回原提案），list_proposals
//! 核对本类型仍待处理的提案（T-182）。两者都要策略平面已接入、能力集授予 `policy.propose_desired`。

use super::{AgentSession, ListProposalsArgs, ProposeArgs, grant::Grant};
use rmcp::ErrorData as McpError;
use studiolo_policy::{PolicyAgentStore, PolicyError, PolicyKind, ProposalInput, audit};

impl AgentSession {
    /// 两件提案工具共同的门槛：授权复核 → 策略平面已接入 → 能力集授予提案能力；不过即拒绝并审计。
    fn proposal_gate(&self, tool: &str) -> Result<(Grant, &PolicyAgentStore), McpError> {
        let grant = self.authorize(tool)?;
        let (Some(store), Some(policy)) = (&self.state.policy, &grant.policy) else {
            self.deny(grant.actor(), tool, "policy_plane_absent");
            return Err(McpError::invalid_request(
                "策略平面未接入（M3 兼容模式）",
                None,
            ));
        };
        if !policy.set.policy.propose_desired {
            self.deny(grant.actor(), tool, "capability_denied");
            return Err(McpError::invalid_request(
                "能力集未授予提交 desired 提案的能力",
                None,
            ));
        }
        Ok((grant, store))
    }

    /// 提交 desired 提案：只入库为 `pending`，永不自动生效；须能力集 `policy.propose_desired`。
    /// 断线重提：本类型在该 subject 最新的未发布草案正是同一份正文且仍待处理时，拿回它
    /// （`deduplicated true`），不新建、不占名额。
    pub(crate) async fn text_propose(&self, args: ProposeArgs) -> Result<String, McpError> {
        let (grant, store) = self.proposal_gate("propose_desired")?;
        let kind = PolicyKind::parse(&args.kind)
            .map_err(|_| McpError::invalid_params("kind 无效", None))?;
        let proposal = store
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
                PolicyError::TooManyProposals => {
                    McpError::invalid_request("待处理提案已达上限，请等待主人处理", None)
                }
                PolicyError::InvalidSubject
                | PolicyError::InvalidKind
                | PolicyError::InvalidBody(_) => McpError::invalid_params(error.to_string(), None),
                _ => McpError::internal_error("策略库不可用，本次拒绝", None),
            })?;
        // 走到这里提案已在库里（新入库的或拿回的）。审计记不上时如实说已入库，不能沿用「本次读写全部
        // 拒绝」：那句会让提交方以为没进库。它仍是本类型在该 subject 最新的待处理草案时，重提会拿回它并补记。
        let event = audit::proposal_submitted(
            audit::Principal::Agent {
                agent_type: &grant.instance.agent_type,
                instance_id: &grant.instance.id,
            },
            &proposal,
        )
        .map_err(|_| McpError::internal_error("审计事件构造失败", None))?;
        self.state.audit(event).map_err(|outage| {
            McpError::internal_error(
                format!(
                    "审计不可用（outage {}）：提案 {} 已入库，但这次提交没能记进审计；恢复后用 list_proposals 核对，或重提同一份正文",
                    outage.outage_id, proposal.record.id
                ),
                None,
            )
        })?;
        let record = &proposal.record;
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
            "desired_id {}\nstatus {}\nok {}\nexpansions {}\nbody_sha256 {}\ndeduplicated {}\n提案永不自动生效；须主人在人侧经 One-time Code 发布。\n{problems}",
            record.id,
            record.status.as_str(),
            report.is_some_and(|report| report.ok),
            report.map_or(0, |report| report.expansions.len()),
            proposal.body_sha256.as_deref().unwrap_or("-"),
            proposal.deduplicated,
        ))
    }

    /// 列本类型仍待处理（pending／validated）的提案，供断线后核对第一次有没有入库；只给摘要列，
    /// 不给正文。按类型列而不按实例：断线重连后是另一个实例。先记读取审计再返回。
    pub(crate) async fn text_list_proposals(
        &self,
        args: ListProposalsArgs,
    ) -> Result<String, McpError> {
        let (grant, store) = self.proposal_gate("list_proposals")?;
        let kind = args
            .kind
            .as_deref()
            .map(PolicyKind::parse)
            .transpose()
            .map_err(|_| McpError::invalid_params("kind 无效", None))?;
        let listing = store
            .open_proposals(&grant.instance.agent_type, kind, args.subject.as_deref())
            .map_err(|_| McpError::internal_error("策略库不可用，本次拒绝供给", None))?;
        self.audit_read(&grant.instance, "policy://desired", "-", "list_proposals")?;
        let lines = listing
            .proposals
            .iter()
            .map(|proposal| {
                format!(
                    "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                    proposal.id,
                    proposal.kind,
                    proposal.subject,
                    proposal.status.as_str(),
                    proposal.body_sha256,
                    proposal.submitted_at,
                    proposal.agent_instance.as_deref().unwrap_or("-"),
                )
            })
            .collect::<Vec<_>>();
        let more = if listing.more {
            "；还有更早的未列出，可按 kind／subject 收窄"
        } else {
            ""
        };
        Ok(format!(
            "# 待处理提案 {} 份（类型 {}，新到旧{more}）\n{}",
            lines.len(),
            grant.instance.agent_type,
            lines.join("\n")
        ))
    }
}
