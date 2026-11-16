//! 仅可信离线恢复装配可调用；不签发身份，不替换因子，不重置审计或猜码预算。

mod audit;
mod state;

use super::{SessionError, SessionManager, recovery, schema};
pub(super) use audit::valid_payload;

/// 恢复撤权的持久结果；不是登录、L3 或服务可上线的授权证明。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoreInvalidation {
    /// 本次恢复撤权提交后的身份版本；相同恢复 ID 返回同一版本。
    pub epoch: i64,
    /// 本次标记为 revoked 的 active/pending 会话数。
    pub revoked_sessions: usize,
    /// 本次移除的旧 L3 证明数。
    pub cleared_l3: usize,
    /// 本次废止的当前恢复码行数（包括已消费行）。
    pub invalidated_recovery_codes: usize,
    /// 两条原恢复事件已由指定账本精确确认；false 时必须保持恢复服务离线。
    pub audit_confirmed: bool,
}

pub(super) fn valid_id(id: &str) -> bool {
    (1..=55).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

impl SessionManager {
    /// 在隔离恢复副本中原子废止全部旧会话、L3 和恢复码，保留当前因子与历史。
    ///
    /// 仅交可信离线装配：调用方必须冻结所有写者并阻止服务监听，不能暴露为 HTTP
    /// 或以此替代 Access/L2/L3。此 API 不检查宿主目录是不是生产目录，也不提供 OS 隔离。
    /// 相同恢复 ID 永久复用原事件；新 ID 消耗两条有界 outbox，不回收幂等锚点。
    /// 审计交付失败不回滚撤权；提交后 I/O 报错可能已生效，先核权威状态，不盲目重试。
    pub fn invalidate_restored_credentials(
        &self,
        restore_id: &str,
    ) -> Result<RestoreInvalidation, SessionError> {
        let _operation = self.enter_operation()?;
        if !valid_id(restore_id) {
            return Err(SessionError::Configuration);
        }
        let prior = self.database.transaction(|db| {
            if schema::version(db)? != 4 {
                return Err(SessionError::MigrationRequired);
            }
            audit::read(db, restore_id)
        })?;
        let (result, dependencies_confirmed) = if let Some(prior) = prior {
            // 原事件可能尚无回执；重试也必须绑定保留因子的原账本，不能换空账本补记。
            let confirmed = self
                .database
                .transaction(|db| recovery::restore_proofs(db, &schema::store_id(db)?))
                .and_then(|proofs| audit::confirm(self, &proofs))
                .is_ok();
            (prior.report, confirmed)
        } else {
            let frozen = self.database.transaction(state::capture)?;
            // 全部原证明在 Auth 锁外重确认；任何错账本/回执在撤权前失败关闭。
            audit::confirm(self, &frozen.originals)?;
            let report = self.database.transaction(|db| {
                // 并发的同 ID 已提交时复用它；不同决定或任一状态变化均不沿用旧快照。
                if let Some(prior) = audit::read(db, restore_id)? {
                    return Ok(prior.report);
                }
                state::commit(db, restore_id, &frozen)
            })?;
            (report, true)
        };
        let audit_confirmed =
            dependencies_confirmed && audit::deliver(self, restore_id).unwrap_or(false);
        Ok(RestoreInvalidation {
            audit_confirmed,
            ..result
        })
    }

    /// 当前身份版本（`auth_meta.epoch`），只读。恢复编排用它核「撤权之后版本严格变大」（T-212）。
    pub fn identity_epoch(&self) -> Result<i64, SessionError> {
        self.database.transaction(|db| {
            db.query_row("SELECT epoch FROM auth_meta WHERE id=1", [], |row| {
                row.get(0)
            })
            .map_err(super::database::required_row)
        })
    }

    /// 这个恢复标识的撤权已经提交过的话，返回当时提交的身份版本；没有返回 `None`，只读。
    /// 恢复编排接手上一次时用它核「这次答复的版本就是当时那次」（T-212）。
    pub fn restored_epoch(&self, restore_id: &str) -> Result<Option<i64>, SessionError> {
        if !valid_id(restore_id) {
            return Ok(None);
        }
        Ok(self
            .database
            .transaction(|db| audit::read(db, restore_id))?
            .map(|prior| prior.report.epoch))
    }

    /// 这个恢复标识的撤权有没有提交、两条恢复事件是不是都已投进账本（都有回执），只读。
    /// 主服务启动闸门用它：新根带着这个标识的 lineage 时，没做完就不许起服务（T-212）。
    /// 不合法的标识一律答「没有」。
    pub fn restore_completed(&self, restore_id: &str) -> Result<bool, SessionError> {
        if !valid_id(restore_id) {
            return Ok(false);
        }
        let prior = self
            .database
            .transaction(|db| audit::read(db, restore_id))?;
        Ok(prior.is_some_and(|prior| {
            prior.events.len() == 2 && prior.events.iter().all(|(_, receipt)| receipt.is_some())
        }))
    }

    /// 这个恢复标识的撤权提交之后，主人有没有重新登录过：有会话在撤权之后的身份版本里激活过（拿到了准入
    /// 回执），只读。之后又退出、过期或被撤的也算登录过。最终切换用它核「主人已重新认证」（T-212）。
    /// 撤权没提交过、不合法的标识一律答「没有」。
    pub fn reauthenticated_after_restore(&self, restore_id: &str) -> Result<bool, SessionError> {
        if !valid_id(restore_id) {
            return Ok(false);
        }
        self.database.transaction(|db| {
            let Some(prior) = audit::read(db, restore_id)? else {
                return Ok(false);
            };
            // 撤权把身份版本加一并撤掉全部旧会话；版本不低于它的会话只能是撤权之后签发的。
            let activated: i64 = db
                .query_row(
                    "SELECT count(*) FROM sessions WHERE epoch>=?1 AND activated_at IS NOT NULL",
                    [prior.report.epoch],
                    |row| row.get(0),
                )
                .map_err(super::database::required_row)?;
            Ok(activated > 0)
        })
    }
}
