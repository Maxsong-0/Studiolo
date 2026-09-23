//! 主人句柄的只读查询：吊销、effective、版本、desired、能力集、规则与 skills。

use crate::{
    PolicyError, PolicyKind,
    model::{CapabilitySet, Skill},
    sql,
};

use super::{
    DesiredRecord, DesiredStatus, EffectiveSummary, PolicyStore, RevocationRecord, VersionMeta,
    VersionRecord, effective_summary, parse_skills,
};

impl PolicyStore {
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
