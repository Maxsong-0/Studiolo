//! Studiolo 策略平面（M4）。
//!
//! 规则母本、skills 与各 AI 类型的能力集三类母本的权威形态：带版本与哈希的私有库
//! `policy.sqlite3`。发布链路固定为 desired → 校验 → diff（权限扩大项标红）→ L3 One-time Code
//! → effective；回滚走同一发布动作带版本号；实例／类型／全局三级吊销记在同一库里，
//! 接入方每次调用前复核。
//!
//! 两个句柄、两种权限：[`PolicyStore`] 是主人发布者（唯一能写版本表与吊销表的句柄），
//! [`PolicyAgentStore`] 是接入方视角（只读 effective 与吊销状态，只能向 desired 表插入
//! `pending` 提案）。提案永不产生授权；L3 校验不在本 crate，由持有主人句柄的组合根在调用
//! 发布前完成。本 crate 不打开网络端口，不持有 canonical Vault 能力。

mod agent;
pub mod audit;
mod database;
mod diff;
mod error;
mod model;
mod sql;
mod storage;
mod store;
mod validate;
pub mod view;

pub use agent::{
    AtEffective, Authorization, CapabilityGrant, CurrentEntries, Denial, PolicyAgentStore,
    ProposalInput,
};
pub use diff::{Diff, DiffEntry, DiffOp};
pub use error::PolicyError;
pub use model::{
    CapabilitySet, HardBinding, MAX_BODY_BYTES, MAX_PROPOSAL_BYTES, PolicyCaps, PolicyKind,
    ReadCaps, Skill, WriteCaps, sha256_hex, validate_rules_subject, validate_skill_name,
    validate_type_name,
};
pub use store::{
    Activation, DesiredInput, DesiredRecord, DesiredStatus, EffectiveEntry, EffectiveSummary,
    MAX_PENDING_PER_INSTANCE, MAX_PENDING_PROPOSALS, PolicyStore, RevocationLevel,
    RevocationRecord, Submitter, VersionMeta, VersionRecord,
};
pub use validate::{Expansion, Problem, Report};
