//! 启动时只查询当前因子锚点的原审计证明；不补写配错的账本或产生探针凭证。

use super::{
    SessionError, SessionManager, outbox, schema,
    totp::{model::Proof, store},
};
use rusqlite::Connection;
use studiolo_audit_ledger::{AuditFilter, AuditPhase};

#[derive(PartialEq, Eq)]
struct Snapshot {
    store_id: String,
    proof: Proof,
}

impl SessionManager {
    /// 预检 v3/v4 当前已确认因子及其在注入账本中的精确双回执，无需读取种子。
    ///
    /// 只在 Auth 事务外按事件与阶段查询，每阶段最多取两条以拒绝歧义，再重验同一证明。
    /// 不追加审计、不签发会话、不消费因子、不迁移或维护；run 仍观察时间和会话到期。
    /// 这是启动快照，不是授权凭据，也不证明所有历史回执均属于该账本。
    ///
    /// v2 返回 MigrationRequired；未登记或仅 pending 返回 FactorUnavailable；
    /// 查询失败或非精确回执返回 AuditUnavailable，期间确认的证明变化返回 Rejected。
    /// 本地损坏、存储失败和时钟错误仍保留原分类；外部错误不会回滚已观察时间。
    pub fn require_http_ready(&self) -> Result<(), SessionError> {
        let original = self.run(|db, _| snapshot(db))?;
        let confirmed = self.confirm_anchor(&original);
        self.run(|db, _| {
            let current = match snapshot(db)? {
                Ok(current) => current,
                Err(error) => return Ok(Err(error)),
            };
            if current != original {
                return Ok(Err(SessionError::Rejected));
            }
            // 即使查询失败，仍提交本次重新观察的地板与会话到期，不把外部错误当作回滚。
            Ok(confirmed)
        })
    }

    fn confirm_anchor(&self, snapshot: &Snapshot) -> Result<(), SessionError> {
        for (phase, original) in [
            (AuditPhase::Intent, &snapshot.proof.intent_receipt),
            (AuditPhase::Outcome, &snapshot.proof.outcome_receipt),
        ] {
            let expected = snapshot.proof.operation.event(&snapshot.store_id, phase)?;
            let records = self
                .audit
                .query(
                    &AuditFilter::default()
                        .with_event_id(expected.event_id().clone())
                        .with_phase(phase)
                        .with_limit(2),
                )
                .map_err(|_| SessionError::AuditUnavailable)?;
            let [record] = records.as_slice() else {
                return Err(SessionError::AuditUnavailable);
            };
            if outbox::receipt(record, &expected)? != *original {
                return Err(SessionError::AuditUnavailable);
            }
        }
        Ok(())
    }
}

fn snapshot(db: &Connection) -> Result<Result<Snapshot, SessionError>, SessionError> {
    if !matches!(schema::version(db)?, 3 | 4) {
        return Ok(Err(SessionError::MigrationRequired));
    }
    let Some(proof) = store::factor(db)?.and_then(|factor| factor.enrollment) else {
        return Ok(Err(SessionError::FactorUnavailable));
    };
    // run 已验证 Enroll/Recovered 与因子、恢复锚点、规范回执的完整本地关联。
    Ok(Ok(Snapshot {
        store_id: schema::store_id(db)?,
        proof,
    }))
}
