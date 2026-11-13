//! 主人发布者句柄：desired → 校验 → 发布／回滚／bootstrap，三级吊销与查询。
//!
//! L3 One-time Code 的校验不在这里；调用方（人侧组合根）必须在调用 [`PolicyStore::activate`]、
//! [`PolicyStore::rollback`] 与 [`PolicyStore::release_global`] 之前完成 `require_l3`。
//! 本句柄不检查调用方是谁——它是「唯一能写版本表的对象」，持有它本身就是主人身份的证明，
//! 因此只能由可信启动层装配，不能进入任何接入方可达的代码路径。

mod activation;
mod desired;
mod queries;
pub(crate) mod queue;
mod revocation;
mod support;
mod types;

use std::path::Path;

use crate::{PolicyError, database::Database, storage::CreatePolicy};

pub use types::{
    Activation, DesiredInput, DesiredRecord, DesiredStatus, EffectiveEntry, EffectiveSummary,
    MAX_PENDING_PER_INSTANCE, MAX_PENDING_PROPOSALS, RevocationLevel, RevocationRecord, Submitter,
    VersionMeta, VersionRecord,
};

// 子模块与接入方句柄（agent.rs）经 `store::` 取用下列内部项。
pub(crate) use support::{effective_summary, enforce_proposal_bounds, parse_skills};
pub(crate) use types::GLOBAL_TARGET;

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
}
