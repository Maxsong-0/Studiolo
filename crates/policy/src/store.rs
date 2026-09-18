//! 主人发布者句柄：desired → 校验 → 发布／回滚／bootstrap，三级吊销与查询。
//!
//! L3 One-time Code 的校验不在这里；调用方（人侧组合根）必须在调用 [`PolicyStore::activate`]、
//! [`PolicyStore::rollback`] 与 [`PolicyStore::release_global`] 之前完成 `require_l3`。
//! 本句柄不检查调用方是谁——它是「唯一能写版本表的对象」，持有它本身就是主人身份的证明，
//! 因此只能由可信启动层装配，不能进入任何接入方可达的代码路径。

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{
    Expansion, PolicyError, PolicyKind, Report,
    database::Database,
    model::{CapabilitySet, MAX_BODY_BYTES, MAX_PROPOSAL_BYTES, Skill, sha256_hex},
    sql,
    storage::CreatePolicy,
    validate,
};

/// 全库待处理接入方提案上限。
pub const MAX_PENDING_PROPOSALS: usize = 256;
/// 每个实例的待处理提案上限。
pub const MAX_PENDING_PER_INSTANCE: usize = 8;

/// desired 的生命周期状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesiredStatus {
    /// 接入方提案，等待主人校验。
    Pending,
    /// 校验通过，可发布。
    Validated,
    /// 校验未通过。
    Rejected,
    /// 已发布为某个版本。
    Activated,
    /// 同 subject 的另一份 desired 已发布，本份作废。
    Superseded,
    /// 主人撤回。
    Withdrawn,
}

impl DesiredStatus {
    /// 库内稳定名。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Validated => "validated",
            Self::Rejected => "rejected",
            Self::Activated => "activated",
            Self::Superseded => "superseded",
            Self::Withdrawn => "withdrawn",
        }
    }

    /// 解析稳定名。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "validated" => Some(Self::Validated),
            "rejected" => Some(Self::Rejected),
            "activated" => Some(Self::Activated),
            "superseded" => Some(Self::Superseded),
            "withdrawn" => Some(Self::Withdrawn),
            _ => None,
        }
    }
}

/// 提交者：主人或某个接入方实例。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Submitter {
    /// 主人（人侧）。
    Owner,
    /// 接入方实例。
    Agent {
        /// 类型名。
        agent_type: String,
        /// 实例 id。
        instance_id: String,
    },
}

/// 一份 desired 输入；`body` 为空表示撤销该 subject。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DesiredInput {
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 正文；`None` = 撤销。
    pub body: Option<String>,
}

/// 库中的一份 desired。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesiredRecord {
    /// id（ULID）。
    pub id: String,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 提交的正文（撤销为空）。
    pub body: Option<String>,
    /// 规范化正文 sha256。
    pub body_sha256: String,
    /// `owner` 或 `agent`。
    pub submitter_kind: String,
    /// 提交方类型（接入方时）。
    pub agent_type: Option<String>,
    /// 提交方实例（接入方时）。
    pub agent_instance: Option<String>,
    /// 提交时间。
    pub submitted_at: String,
    /// 状态。
    pub status: DesiredStatus,
    /// 最近一次校验报告（损坏时为空）。
    pub report: Option<Report>,
    /// 最近一次校验时间。
    pub validated_at: Option<String>,
    /// 发布成的版本号。
    pub activated_version: Option<u64>,
}

/// 一次发布的结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Activation {
    /// 新版本号。
    pub version: u64,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 是否撤销。
    pub retired: bool,
    /// 正文 sha256。
    pub body_sha256: String,
    /// 发布后的 effective 哈希。
    pub effective_hash: String,
    /// 发布前的 effective 哈希。
    pub previous_effective_hash: String,
    /// 本次扩大项。
    pub expansions: Vec<Expansion>,
    /// 来源 desired。
    pub desired_id: Option<String>,
    /// 回滚到的历史版本。
    pub rollback_of: Option<u64>,
    /// 本次是否顺带解除了该类型的吊销。
    pub cleared_type_revocation: bool,
    /// 发布者（`owner` / `bootstrap`）。
    pub activated_by: String,
}

/// 版本元信息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionMeta {
    /// 版本号。
    pub version: u64,
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 是否撤销。
    pub retired: bool,
    /// 正文 sha256。
    pub body_sha256: String,
    /// 发布后的 effective 哈希。
    pub effective_hash: String,
    /// 发布前的 effective 哈希。
    pub previous_effective_hash: String,
    /// 发布时间。
    pub activated_at: String,
    /// 发布者。
    pub activated_by: String,
    /// 来源 desired。
    pub desired_id: Option<String>,
    /// 回滚到的历史版本。
    pub rollback_of: Option<u64>,
    /// 发布时记录的扩大项。
    pub expansions: Vec<Expansion>,
}

/// 版本元信息加正文。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionRecord {
    /// 元信息。
    pub meta: VersionMeta,
    /// 正文（撤销版本为空）。
    pub body: Option<String>,
}

/// effective 中的一项。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveEntry {
    /// 类别。
    pub kind: PolicyKind,
    /// subject。
    pub subject: String,
    /// 版本。
    pub version: u64,
    /// 正文 sha256。
    pub body_sha256: String,
    /// 发布时间。
    pub activated_at: String,
}

/// effective 摘要。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveSummary {
    /// 当前 effective 哈希。
    pub effective_hash: String,
    /// 现行各项。
    pub entries: Vec<EffectiveEntry>,
    /// 是否处于全局吊销。
    pub global_revoked: bool,
    /// 待处理 desired 数（pending ＋ validated）。
    pub pending_desired: usize,
}

/// 吊销级别。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevocationLevel {
    /// 单个实例（不可恢复）。
    Instance,
    /// 整个类型（重新发布该类型能力集即恢复）。
    Type,
    /// 全局（L3 解除）。
    Global,
}

impl RevocationLevel {
    /// 库内稳定名。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Instance => "instance",
            Self::Type => "type",
            Self::Global => "global",
        }
    }

    /// 解析稳定名。
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "instance" => Some(Self::Instance),
            "type" => Some(Self::Type),
            "global" => Some(Self::Global),
            _ => None,
        }
    }
}

/// 一条吊销记录。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationRecord {
    /// 序号。
    pub seq: u64,
    /// 级别。
    pub level: RevocationLevel,
    /// 目标（实例 id／类型名／`*`）。
    pub target: String,
    /// 吊销时间。
    pub revoked_at: String,
    /// 吊销者。
    pub revoked_by: String,
    /// 理由。
    pub reason: String,
    /// 解除时间。
    pub released_at: Option<String>,
    /// 解除者。
    pub released_by: Option<String>,
}

/// 全局吊销的固定目标名。
pub(crate) const GLOBAL_TARGET: &str = "*";

/// 主人发布者句柄。
pub struct PolicyStore {
    db: Database,
}

impl PolicyStore {
    /// 打开部署批准的绝对私有目录；仅初始化本次独占新建的 `policy.sqlite3`。
    pub fn open(private_directory: &Path) -> Result<Self, PolicyError> {
        Ok(Self {
            db: Database::open(private_directory, CreatePolicy::AllowNew)?,
        })
    }

    /// 只打开并验证已有库；缺失返回 `MissingStore`。
    pub fn open_existing(private_directory: &Path) -> Result<Self, PolicyError> {
        Ok(Self {
            db: Database::open(private_directory, CreatePolicy::ExistingOnly)?,
        })
    }

    /// 提交一份 desired：校验、与 effective 做 diff、入库。
    ///
    /// 主人提交的记为 `validated` 或 `rejected`；接入方提交的记为 `pending`（结构无效则
    /// `rejected`），两者都永不自动生效。
    pub fn submit_desired(
        &self,
        input: DesiredInput,
        submitter: Submitter,
        now: &str,
    ) -> Result<DesiredRecord, PolicyError> {
        crate::model::validate_subject(input.kind, &input.subject)?;
        let id = ulid::Ulid::generate().to_string();
        self.db.write(|tx| {
            let (limit, submitter_kind, agent_type, instance) = match &submitter {
                Submitter::Owner => (MAX_BODY_BYTES, "owner", None, None),
                Submitter::Agent {
                    agent_type,
                    instance_id,
                } => {
                    if input.body.is_none() {
                        return Err(PolicyError::InvalidBody("接入方不能提交撤销"));
                    }
                    enforce_proposal_bounds(tx, instance_id)?;
                    (
                        MAX_PROPOSAL_BYTES,
                        "agent",
                        Some(agent_type.as_str()),
                        Some(instance_id.as_str()),
                    )
                }
            };
            let context = sql::context(tx, input.kind, &input.subject)?;
            let report = validate::validate(
                input.kind,
                &input.subject,
                input.body.as_deref(),
                limit,
                &context,
            );
            let status = match (&submitter, report.ok) {
                (_, false) => DesiredStatus::Rejected,
                (Submitter::Owner, true) => DesiredStatus::Validated,
                (Submitter::Agent { .. }, true) => DesiredStatus::Pending,
            };
            // 入库的是提交原文而不是规范化正文：结构无效的 desired 必须原样保留，
            // 重新校验才会得到同样的拒绝，绝不能因为规范化失败而退化成「撤销」。
            sql::insert_desired(
                tx,
                &id,
                &report,
                input.body.as_deref(),
                submitter_kind,
                agent_type,
                instance,
                now,
                status,
            )?;
            sql::desired(tx, &id)?.ok_or(PolicyError::StoreUnavailable)
        })
    }

    /// 对一份 desired 重新校验（针对当前 effective），更新状态与报告。
    pub fn validate_desired(&self, id: &str, now: &str) -> Result<DesiredRecord, PolicyError> {
        self.db.write(|tx| {
            let record = sql::desired(tx, id)?.ok_or(PolicyError::DesiredNotFound)?;
            if !matches!(
                record.status,
                DesiredStatus::Pending | DesiredStatus::Validated | DesiredStatus::Rejected
            ) {
                return Err(PolicyError::DesiredNotActivatable(
                    record.status.as_str().to_owned(),
                ));
            }
            let context = sql::context(tx, record.kind, &record.subject)?;
            let report = validate::validate(
                record.kind,
                &record.subject,
                record.body.as_deref(),
                MAX_BODY_BYTES,
                &context,
            );
            let status = if report.ok {
                DesiredStatus::Validated
            } else {
                DesiredStatus::Rejected
            };
            sql::update_desired(tx, id, status, Some(&report), now, None)?;
            sql::desired(tx, id)?.ok_or(PolicyError::StoreUnavailable)
        })
    }

    /// 主人撤回一份尚未发布的 desired。
    pub fn withdraw_desired(&self, id: &str, now: &str) -> Result<DesiredRecord, PolicyError> {
        self.db.write(|tx| {
            let record = sql::desired(tx, id)?.ok_or(PolicyError::DesiredNotFound)?;
            if !matches!(
                record.status,
                DesiredStatus::Pending | DesiredStatus::Validated | DesiredStatus::Rejected
            ) {
                return Err(PolicyError::DesiredNotActivatable(
                    record.status.as_str().to_owned(),
                ));
            }
            sql::update_desired(tx, id, DesiredStatus::Withdrawn, None, now, None)?;
            sql::desired(tx, id)?.ok_or(PolicyError::StoreUnavailable)
        })
    }

    /// 发布一份 desired（调用前必须已过 L3）：CAS 校验 effective 哈希 → 重新校验 → 写新版本。
    pub fn activate(
        &self,
        id: &str,
        expected_effective_hash: &str,
        actor: &str,
        now: &str,
    ) -> Result<Activation, PolicyError> {
        self.db.write(|tx| {
            check_hash(tx, expected_effective_hash)?;
            let record = sql::desired(tx, id)?.ok_or(PolicyError::DesiredNotFound)?;
            if !matches!(
                record.status,
                DesiredStatus::Pending | DesiredStatus::Validated | DesiredStatus::Rejected
            ) {
                return Err(PolicyError::DesiredNotActivatable(
                    record.status.as_str().to_owned(),
                ));
            }
            let context = sql::context(tx, record.kind, &record.subject)?;
            let report = validate::validate(
                record.kind,
                &record.subject,
                record.body.as_deref(),
                MAX_BODY_BYTES,
                &context,
            );
            if !report.ok {
                sql::update_desired(tx, id, DesiredStatus::Rejected, Some(&report), now, None)?;
                return Err(PolicyError::ValidationFailed);
            }
            let activation = publish(tx, &report, actor, now, Some(id), None)?;
            sql::update_desired(
                tx,
                id,
                DesiredStatus::Activated,
                Some(&report),
                now,
                Some(activation.version),
            )?;
            tx.execute(
                "UPDATE policy_desired SET status = 'superseded'
                 WHERE kind = ?1 AND subject = ?2 AND id != ?3
                   AND status IN ('pending', 'validated', 'rejected')",
                rusqlite::params![record.kind.as_str(), record.subject, id],
            )
            .map_err(crate::database::unavailable)?;
            Ok(activation)
        })
    }

    /// 回滚到历史版本（调用前必须已过 L3）：新建一个正文等于该版本的版本，不改历史。
    pub fn rollback(
        &self,
        version: u64,
        expected_effective_hash: &str,
        actor: &str,
        now: &str,
    ) -> Result<Activation, PolicyError> {
        self.db.write(|tx| {
            check_hash(tx, expected_effective_hash)?;
            let target = sql::version_record(tx, version)?.ok_or(PolicyError::VersionNotFound)?;
            let context = sql::context(tx, target.meta.kind, &target.meta.subject)?;
            let report = validate::validate(
                target.meta.kind,
                &target.meta.subject,
                target.body.as_deref(),
                MAX_BODY_BYTES,
                &context,
            );
            if !report.ok {
                return Err(PolicyError::ValidationFailed);
            }
            publish(tx, &report, actor, now, None, Some(version))
        })
    }

    /// 首版 bootstrap：只允许为从未有过版本的 subject 写 version 1（可信本地 CLI 使用）。
    pub fn bootstrap(&self, input: DesiredInput, now: &str) -> Result<Activation, PolicyError> {
        crate::model::validate_subject(input.kind, &input.subject)?;
        self.db.write(|tx| {
            let existed: i64 = tx
                .query_row(
                    "SELECT count(*) FROM policy_version WHERE kind = ?1 AND subject = ?2",
                    rusqlite::params![input.kind.as_str(), input.subject],
                    |row| row.get(0),
                )
                .map_err(crate::database::unavailable)?;
            if existed != 0 {
                return Err(PolicyError::AlreadyBootstrapped);
            }
            let body = input
                .body
                .as_deref()
                .ok_or(PolicyError::InvalidBody("bootstrap 不能撤销"))?;
            let context = sql::context(tx, input.kind, &input.subject)?;
            let report = validate::validate(
                input.kind,
                &input.subject,
                Some(body),
                MAX_BODY_BYTES,
                &context,
            );
            if !report.ok {
                return Err(PolicyError::ValidationFailed);
            }
            publish(tx, &report, "bootstrap", now, None, None)
        })
    }

    /// 对一份输入做校验并返回报告，不入库（供界面预览与 CLI 自检）。
    pub fn preview(&self, input: &DesiredInput) -> Result<Report, PolicyError> {
        crate::model::validate_subject(input.kind, &input.subject)?;
        self.db.read(|tx| {
            let context = sql::context(tx, input.kind, &input.subject)?;
            Ok(validate::validate(
                input.kind,
                &input.subject,
                input.body.as_deref(),
                MAX_BODY_BYTES,
                &context,
            ))
        })
    }

    /// 吊销（安全方向，L2）；已处于吊销中时返回 `false`。
    pub fn revoke(
        &self,
        level: RevocationLevel,
        target: &str,
        actor: &str,
        reason: &str,
        now: &str,
    ) -> Result<bool, PolicyError> {
        let target = match level {
            RevocationLevel::Global => GLOBAL_TARGET.to_owned(),
            RevocationLevel::Type => {
                crate::model::validate_type_name(target)?;
                target.to_owned()
            }
            RevocationLevel::Instance => {
                if target.is_empty()
                    || target.len() > 64
                    || target.chars().any(|c| c.is_control() || c.is_whitespace())
                {
                    return Err(PolicyError::InvalidSubject);
                }
                target.to_owned()
            }
        };
        self.db.write(|tx| {
            if sql::revoked(tx, level, &target)? {
                return Ok(false);
            }
            tx.execute(
                "INSERT INTO policy_revocation (level, target, revoked_at, revoked_by, reason)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![level.as_str(), target, now, actor, reason],
            )
            .map_err(crate::database::unavailable)?;
            Ok(true)
        })
    }

    /// 解除全局吊销（调用前必须已过 L3）。
    pub fn release_global(&self, actor: &str, now: &str) -> Result<(), PolicyError> {
        self.db.write(|tx| {
            let changed = tx
                .execute(
                    "UPDATE policy_revocation SET released_at = ?1, released_by = ?2
                     WHERE level = 'global' AND released_at IS NULL",
                    rusqlite::params![now, actor],
                )
                .map_err(crate::database::unavailable)?;
            if changed == 0 {
                return Err(PolicyError::NotRevoked);
            }
            Ok(())
        })
    }

    /// 吊销记录（`active_only` 只看生效中的）。
    pub fn revocations(&self, active_only: bool) -> Result<Vec<RevocationRecord>, PolicyError> {
        self.db.read(|tx| sql::revocations(tx, active_only))
    }

    /// effective 摘要。
    pub fn effective(&self) -> Result<EffectiveSummary, PolicyError> {
        self.db.read(effective_summary)
    }

    /// 某个版本（含正文）。
    pub fn version(&self, version: u64) -> Result<Option<VersionRecord>, PolicyError> {
        self.db.read(|tx| sql::version_record(tx, version))
    }

    /// 版本列表（新到旧）。
    pub fn versions(
        &self,
        kind: Option<PolicyKind>,
        subject: Option<&str>,
        limit: usize,
    ) -> Result<Vec<VersionMeta>, PolicyError> {
        self.db.read(|tx| sql::versions(tx, kind, subject, limit))
    }

    /// 一份 desired。
    pub fn desired(&self, id: &str) -> Result<Option<DesiredRecord>, PolicyError> {
        self.db.read(|tx| sql::desired(tx, id))
    }

    /// desired 列表（新到旧）。
    pub fn desired_list(
        &self,
        status: Option<DesiredStatus>,
        limit: usize,
    ) -> Result<Vec<DesiredRecord>, PolicyError> {
        self.db.read(|tx| sql::desired_list(tx, status, limit))
    }

    /// 某类型现行能力集。
    pub fn capability_set(
        &self,
        ai_type: &str,
    ) -> Result<Option<(CapabilitySet, VersionMeta)>, PolicyError> {
        self.db.read(|tx| sql::capability_set(tx, ai_type))
    }

    /// 现行规则母本（按 subject 排序）。
    pub fn rules(&self) -> Result<Vec<VersionRecord>, PolicyError> {
        self.db
            .read(|tx| sql::current_records(tx, PolicyKind::Rules))
    }

    /// 现行 skills。
    pub fn skills(&self) -> Result<Vec<(Skill, VersionMeta)>, PolicyError> {
        self.db
            .read(|tx| parse_skills(sql::current_records(tx, PolicyKind::Skills)?))
    }
}

pub(crate) fn parse_skills(
    records: Vec<VersionRecord>,
) -> Result<Vec<(Skill, VersionMeta)>, PolicyError> {
    records
        .into_iter()
        .map(|record| {
            let body = record.body.ok_or(PolicyError::CorruptStore)?;
            let skill =
                serde_json::from_str::<Skill>(&body).map_err(|_| PolicyError::CorruptStore)?;
            Ok((skill, record.meta))
        })
        .collect()
}

pub(crate) fn effective_summary(
    tx: &rusqlite::Transaction<'_>,
) -> Result<EffectiveSummary, PolicyError> {
    let pending: i64 = tx
        .query_row(
            "SELECT count(*) FROM policy_desired WHERE status IN ('pending', 'validated')",
            [],
            |row| row.get(0),
        )
        .map_err(crate::database::unavailable)?;
    Ok(EffectiveSummary {
        effective_hash: sql::effective_hash(tx)?,
        entries: sql::current_entries(tx)?,
        global_revoked: sql::revoked(tx, RevocationLevel::Global, GLOBAL_TARGET)?,
        pending_desired: pending.max(0) as usize,
    })
}

pub(crate) fn enforce_proposal_bounds(
    tx: &rusqlite::Transaction<'_>,
    instance_id: &str,
) -> Result<(), PolicyError> {
    let per_instance: i64 = tx
        .query_row(
            "SELECT count(*) FROM policy_desired WHERE agent_instance = ?1 AND status = 'pending'",
            rusqlite::params![instance_id],
            |row| row.get(0),
        )
        .map_err(crate::database::unavailable)?;
    let total: i64 = tx
        .query_row(
            "SELECT count(*) FROM policy_desired WHERE submitter_kind = 'agent' AND status = 'pending'",
            [],
            |row| row.get(0),
        )
        .map_err(crate::database::unavailable)?;
    if per_instance >= MAX_PENDING_PER_INSTANCE as i64 || total >= MAX_PENDING_PROPOSALS as i64 {
        return Err(PolicyError::TooManyProposals);
    }
    Ok(())
}

fn check_hash(tx: &rusqlite::Transaction<'_>, expected: &str) -> Result<(), PolicyError> {
    let current = sql::effective_hash(tx)?;
    if current != expected {
        return Err(PolicyError::HashMismatch { current });
    }
    Ok(())
}

fn publish(
    tx: &rusqlite::Transaction<'_>,
    report: &Report,
    actor: &str,
    now: &str,
    desired_id: Option<&str>,
    rollback_of: Option<u64>,
) -> Result<Activation, PolicyError> {
    let previous = sql::effective_hash(tx)?;
    let body = report.canonical.clone().unwrap_or_default();
    let body_sha256 = if report.retire {
        sha256_hex(b"")
    } else {
        report.body_sha256.clone()
    };
    let version = sql::insert_version(
        tx,
        report.kind,
        &report.subject,
        &body,
        &body_sha256,
        report.retire,
        &previous,
        now,
        actor,
        desired_id,
        rollback_of,
        &report.expansions,
    )?;
    let effective_hash = sql::effective_hash(tx)?;
    let cleared_type_revocation = if report.kind == PolicyKind::CapabilitySet && !report.retire {
        tx.execute(
            "UPDATE policy_revocation SET released_at = ?1, released_by = ?2
             WHERE level = 'type' AND target = ?3 AND released_at IS NULL",
            rusqlite::params![now, actor, report.subject],
        )
        .map_err(crate::database::unavailable)?
            > 0
    } else {
        false
    };
    Ok(Activation {
        version,
        kind: report.kind,
        subject: report.subject.clone(),
        retired: report.retire,
        body_sha256,
        effective_hash,
        previous_effective_hash: previous,
        expansions: report.expansions.clone(),
        desired_id: desired_id.map(str::to_owned),
        rollback_of,
        cleared_type_revocation,
        activated_by: actor.to_owned(),
    })
}
