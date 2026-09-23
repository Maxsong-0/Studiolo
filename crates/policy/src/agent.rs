//! 接入方视角句柄：读 effective 与吊销状态、只能插入 `pending` 提案。
//!
//! 它与主人句柄打开同一个 `policy.sqlite3`，但代码路径里没有任何写版本表或吊销表的语句；
//! 提案入库不改变 effective，不产生授权。

use std::path::Path;

use crate::{
    DesiredRecord, DesiredStatus, EffectiveSummary, PolicyError, PolicyKind, Skill, VersionMeta,
    VersionRecord,
    database::Database,
    model::{CapabilitySet, MAX_PROPOSAL_BYTES},
    sql,
    storage::CreatePolicy,
    store::{self, GLOBAL_TARGET, RevocationLevel},
    validate,
};

/// 接入方在本次调用被拒的结构化原因（进 `agent.action_denied` 的 reason）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Denial {
    /// 全局吊销生效中。
    GlobalRevoked,
    /// 该类型被吊销。
    TypeRevoked,
    /// 该实例被吊销。
    InstanceRevoked,
    /// 该类型没有生效的能力集（未登记者零权限）。
    NoCapabilitySet,
}

impl Denial {
    /// 审计 reason 的稳定名。
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GlobalRevoked => "global_revoked",
            Self::TypeRevoked => "type_revoked",
            Self::InstanceRevoked => "instance_revoked",
            Self::NoCapabilitySet => "no_capability_set",
        }
    }
}

/// 某类型现行能力集及其版本。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityGrant {
    /// 能力集。
    pub set: CapabilitySet,
    /// 版本元信息（含发布时 effective 哈希）。
    pub meta: VersionMeta,
}

/// 与读取时 effective 哈希绑在一起的读取结果。
///
/// 二者取自同一个读事务：人侧进程在读取途中发布新版本，也不会让「策略版本」与条目各属一版。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AtEffective<T> {
    /// 读取时的 effective 哈希。
    pub effective_hash: String,
    /// 该哈希下的读取结果。
    pub value: T,
}

/// 路牌要的现行规则母本与 skills。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CurrentEntries {
    /// 现行规则母本（按 subject 排序）。
    pub rules: Vec<VersionRecord>,
    /// 现行 skills。
    pub skills: Vec<(Skill, VersionMeta)>,
}

/// 一次授权复核的结果：要么拿到能力集，要么带结构化拒绝原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorization {
    /// 复核时的 effective 哈希。
    pub effective_hash: String,
    /// 能力集（被拒时为空）。
    pub grant: Option<CapabilityGrant>,
    /// 拒绝原因（通过时为空）。
    pub denial: Option<Denial>,
}

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

/// 接入方句柄。
pub struct PolicyAgentStore {
    db: Database,
}

impl PolicyAgentStore {
    /// 只打开并验证已有库；缺失返回 `MissingStore`。
    pub fn open_existing(private_directory: &Path) -> Result<Self, PolicyError> {
        Ok(Self {
            db: Database::open(private_directory, CreatePolicy::ExistingOnly)?,
        })
    }

    /// 每次调用前复核：全局吊销 → 类型吊销 → 实例吊销 → 能力集；同一快照内读取。
    pub fn authorization(
        &self,
        agent_type: &str,
        instance_id: Option<&str>,
    ) -> Result<Authorization, PolicyError> {
        self.db.read(|tx| {
            let effective_hash = sql::effective_hash(tx)?;
            let denial = if sql::revoked(tx, RevocationLevel::Global, GLOBAL_TARGET)? {
                Some(Denial::GlobalRevoked)
            } else if sql::revoked(tx, RevocationLevel::Type, agent_type)? {
                Some(Denial::TypeRevoked)
            } else if let Some(instance) = instance_id
                && sql::revoked(tx, RevocationLevel::Instance, instance)?
            {
                Some(Denial::InstanceRevoked)
            } else {
                None
            };
            if let Some(denial) = denial {
                return Ok(Authorization {
                    effective_hash,
                    grant: None,
                    denial: Some(denial),
                });
            }
            match sql::capability_set(tx, agent_type)? {
                Some((set, meta)) => Ok(Authorization {
                    effective_hash,
                    grant: Some(CapabilityGrant { set, meta }),
                    denial: None,
                }),
                None => Ok(Authorization {
                    effective_hash,
                    grant: None,
                    denial: Some(Denial::NoCapabilitySet),
                }),
            }
        })
    }

    /// 接入方提交 desired 提案：有界、只入 `pending`（结构无效则 `rejected`），永不生效。
    pub fn propose(
        &self,
        agent_type: &str,
        instance_id: &str,
        input: ProposalInput,
        now: &str,
    ) -> Result<DesiredRecord, PolicyError> {
        crate::model::validate_subject(input.kind, &input.subject)?;
        let id = ulid::Ulid::generate().to_string();
        self.db.write(|tx| {
            store::enforce_proposal_bounds(tx, instance_id)?;
            let context = sql::context(tx, input.kind, &input.subject)?;
            let report = validate::validate(
                input.kind,
                &input.subject,
                Some(&input.body),
                MAX_PROPOSAL_BYTES,
                &context,
            );
            let status = if report.ok {
                DesiredStatus::Pending
            } else {
                DesiredStatus::Rejected
            };
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
            sql::desired(tx, &id)?.ok_or(PolicyError::StoreUnavailable)
        })
    }

    /// effective 摘要。
    pub fn effective(&self) -> Result<EffectiveSummary, PolicyError> {
        self.db.read(store::effective_summary)
    }

    /// 现行规则母本（按 subject 排序），连同同一读事务里的 effective 哈希。
    pub fn rules(&self) -> Result<AtEffective<Vec<VersionRecord>>, PolicyError> {
        self.db.read(|tx| {
            Ok(AtEffective {
                effective_hash: sql::effective_hash(tx)?,
                value: sql::current_records(tx, PolicyKind::Rules)?,
            })
        })
    }

    /// 现行 skills，连同同一读事务里的 effective 哈希。
    pub fn skills(&self) -> Result<AtEffective<Vec<(Skill, VersionMeta)>>, PolicyError> {
        self.db.read(|tx| {
            Ok(AtEffective {
                effective_hash: sql::effective_hash(tx)?,
                value: store::parse_skills(sql::current_records(tx, PolicyKind::Skills)?)?,
            })
        })
    }

    /// 路牌用：现行规则母本与 skills 一并取出，三者同属一个读事务。
    pub fn rules_and_skills(&self) -> Result<AtEffective<CurrentEntries>, PolicyError> {
        self.db.read(|tx| {
            Ok(AtEffective {
                effective_hash: sql::effective_hash(tx)?,
                value: CurrentEntries {
                    rules: sql::current_records(tx, PolicyKind::Rules)?,
                    skills: store::parse_skills(sql::current_records(tx, PolicyKind::Skills)?)?,
                },
            })
        })
    }

    /// 某个版本（含正文）。
    pub fn version(&self, version: u64) -> Result<Option<VersionRecord>, PolicyError> {
        self.db.read(|tx| sql::version_record(tx, version))
    }
}
