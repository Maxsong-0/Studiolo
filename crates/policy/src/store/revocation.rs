//! 三级吊销（安全方向）与全局吊销解除（须先过 L3）。

use crate::{PolicyError, sql};

use super::{GLOBAL_TARGET, PolicyStore, RevocationLevel};

impl PolicyStore {
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
}
