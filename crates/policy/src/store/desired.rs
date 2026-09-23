//! desired 的提交、复核、撤回与不入库预览。

use crate::{
    PolicyError, Report,
    model::{MAX_BODY_BYTES, MAX_PROPOSAL_BYTES},
    sql, validate,
};

use super::{
    DesiredInput, DesiredRecord, DesiredStatus, PolicyStore, Submitter, enforce_proposal_bounds,
};

impl PolicyStore {
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
}
