//! 接入方提案：入库（两个句柄共用这一处，含断线重提）与本类型仍待处理提案的列表（T-182）。
//!
//! 断线重提：提交方调用途中断线（例如服务端重启），无从知道第一次有没有入库，重连后是另一个实例。
//! 本类型在同一 kind/subject 上最新的未发布草案若正是同一规范化正文、仍待处理的那份，再提就原样拿回它，
//! 不新建、不占名额；本类型之后又提过别的正文时照常新建，让这份正文重新成为本类型最新的提议。
//! 只看本类型自己的草案，与发布的拦截同一口径（同一提交者有更新的未发布草案才挡，T-181）：主人和
//! 别的类型的更新草案既不挡这份的发布，也不让重提多出一条；重提的结果也就不透露别人有没有新草案。
//! 判定与插入同在一个写事务里。

use rusqlite::Transaction;

use crate::{
    DesiredRecord, DesiredStatus, PolicyError, PolicyKind,
    model::MAX_PROPOSAL_BYTES,
    sql,
    store::{enforce_proposal_bounds, queue},
    validate,
};

/// `list_proposals` 一次最多列出的提案份数。
pub const MAX_LISTED_PROPOSALS: usize = 64;

/// 接入方提案输入；接入方不能撤销任何 subject。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalInput {
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 正文。
    pub body: String,
}

/// 一次提交的结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// 新入库的，或断线重提拿回的已有提案（原样，不改任何字段）。
    pub record: DesiredRecord,
    /// 服务端算的本次正文规范化摘要（规则母本 CRLF 换 LF，其余为规范 JSON）；
    /// 过不了规范化（结构无效）时为空，这样的正文每次都新建一条 `rejected`，不与任何提案相认。
    pub body_sha256: Option<String>,
    /// 是否拿回了已有提案（没有新建，也没有占名额）。
    pub deduplicated: bool,
}

/// 列表里的一份提案：只有核对用的摘要列，不含正文与校验报告。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalSummary {
    /// desired id。
    pub id: String,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// `pending` 或 `validated`。
    pub status: DesiredStatus,
    /// 规范化正文 sha256。
    pub body_sha256: String,
    /// 提交时间。
    pub submitted_at: String,
    /// 提交它的实例。
    pub agent_instance: Option<String>,
}

/// 某类型仍待处理的提案，新到旧（按入库先后）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProposalListing {
    /// 至多 [`MAX_LISTED_PROPOSALS`] 份。
    pub proposals: Vec<ProposalSummary>,
    /// 超出上限，还有更早的没有列出。
    pub more: bool,
}

/// 接入方提案入库：断线重提拿回已有提案；否则有界、只入 `pending`（结构无效则 `rejected`），永不生效。
pub(crate) fn submit(
    tx: &Transaction<'_>,
    agent_type: &str,
    instance_id: &str,
    input: &ProposalInput,
    now: &str,
) -> Result<Proposal, PolicyError> {
    let context = sql::context(tx, input.kind, &input.subject)?;
    let report = validate::validate(
        input.kind,
        &input.subject,
        Some(&input.body),
        MAX_PROPOSAL_BYTES,
        &context,
    );
    // 摘要只认校验时规范化出来的那份：与入库的 body_sha256 同出一处。
    let body_sha256 = report
        .canonical
        .is_some()
        .then(|| report.body_sha256.clone());
    // 先认重提、后查名额：名额满时重提照样拿回原提案。
    if let Some(digest) = &body_sha256
        && let Some(record) = retried(tx, agent_type, input, digest)?
    {
        return Ok(Proposal {
            record,
            body_sha256,
            deduplicated: true,
        });
    }
    enforce_proposal_bounds(tx, instance_id)?;
    let status = if report.ok {
        DesiredStatus::Pending
    } else {
        DesiredStatus::Rejected
    };
    let id = ulid::Ulid::generate().to_string();
    // 入库的是提交原文而不是规范化正文：结构无效的提案原样保留，重新校验才会得到同样的拒绝。
    sql::insert_desired(
        tx,
        &id,
        &report,
        Some(&input.body),
        "agent",
        Some(agent_type),
        Some(instance_id),
        now,
        status,
    )?;
    Ok(Proposal {
        record: sql::desired(tx, &id)?.ok_or(PolicyError::StoreUnavailable)?,
        body_sha256,
        deduplicated: false,
    })
}

/// 本类型在该 subject 最新的未发布草案若是同一正文、仍待处理的提案，就是这次重提要拿回的那份。
/// 「最新」与发布的拦截出自同一处（`store::queue`）。被拒（rejected）的不拿回：重提应当再校验一次。
fn retried(
    tx: &Transaction<'_>,
    agent_type: &str,
    input: &ProposalInput,
    digest: &str,
) -> Result<Option<DesiredRecord>, PolicyError> {
    let Some(id) = queue::latest_open_from_agent(tx, agent_type, input.kind, &input.subject)?
    else {
        return Ok(None);
    };
    let record = sql::desired(tx, &id)?.ok_or(PolicyError::StoreUnavailable)?;
    let same = matches!(
        record.status,
        DesiredStatus::Pending | DesiredStatus::Validated
    ) && record.body_sha256 == digest;
    Ok(same.then_some(record))
}

/// 本类型仍待处理（pending／validated）的接入方提案，可按 kind／subject 收窄。按类型而不按实例列：
/// 断线重连后是另一个实例，仍须看得到断线前提交的那份。
pub(crate) fn open(
    tx: &Transaction<'_>,
    agent_type: &str,
    kind: Option<PolicyKind>,
    subject: Option<&str>,
) -> Result<ProposalListing, PolicyError> {
    // 多取一份，用来判断还有没有更早的。
    let mut proposals =
        sql::open_agent_proposals(tx, agent_type, kind, subject, MAX_LISTED_PROPOSALS + 1)?;
    let more = proposals.len() > MAX_LISTED_PROPOSALS;
    proposals.truncate(MAX_LISTED_PROPOSALS);
    Ok(ProposalListing { proposals, more })
}
