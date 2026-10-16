//! 发布、回滚与首版 bootstrap：写新版本的三条路径（发布与回滚须先过 L3）。

use crate::{PolicyError, model::MAX_BODY_BYTES, sql, validate};

use super::{
    Activation, DesiredInput, DesiredStatus, PolicyStore, queue,
    support::{check_hash, publish},
};

impl PolicyStore {
    /// 发布一份 desired（调用前必须已过 L3）：CAS 校验 effective 哈希 → 同一提交者有更晚入库的
    /// 未发布 desired 就不发 → 重新校验 → 写新版本 → 同一对象比它早入库的未发布 desired 标为
    /// 已取代，比它晚的留着待处理。
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
            // 同一提交者还有更晚入库的未发布 desired 就不发这份（T-181）：要发这份，先撤回更新的。
            // 不同提交者互不阻挡（用户 2026-10-16 定），AI 交新提案挡不住主人发布。
            let placement = queue::place(tx, record.kind, &record.subject, id)?;
            if !placement.blocked_by.is_empty() {
                return Err(PolicyError::NewerDesiredFromSameSubmitter {
                    newer: placement.blocked_by,
                });
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
            // 只取代比它早入库的；比它晚的（别的提交者交的）留着待处理，不悄悄作废谁的提议。
            queue::supersede(tx, &placement.older)?;
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
}
