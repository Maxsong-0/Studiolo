//! 有界终态回收；跨库重试仅确认同一计划，绝不授予或重建会话。

mod candidate;
mod checkpoint;
mod commit;
mod plan;
mod proof;
mod selection;

use super::{SessionError, SessionManager, database::unavailable, schema};
use rusqlite::Connection;
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};

const BATCH: usize = 16;
const MAX_PLAN_BYTES: usize = 32 * 1024;

/// 本次可信维护返回的非秘密统计，不是授权证明。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RetentionReport {
    /// 本调用最终事务实际删除的终态会话数；迟到重试不会重复计数。
    pub reclaimed_sessions: usize,
    /// 调度提示；调用者不得把它解释为仍有有效权限。
    pub remaining: RetentionWork,
}

/// 下一轮维护是否仍有工作；不提供任意对象删除入口。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionWork {
    /// 当前没有终态或可推进的身份版本前缀。
    None,
    /// 当前仍有可继续处理的有界工作。
    More,
    /// 终态证据尚未全部交付，不能清理。
    AwaitingDelivery,
}

impl SessionManager {
    /// 最多回收 16 个终态并推进 16 个 epoch；审计调用不持 Auth 写锁。
    pub fn maintain_retention(&self) -> Result<RetentionReport, SessionError> {
        self.database.transaction(|db| {
            if !matches!(schema::version(db)?, 2..=4) {
                return Err(SessionError::UnsupportedSchema);
            }
            Ok(())
        })?;
        self.flush_audit(64)?;
        let frozen = self.database.transaction(selection::obtain)?;
        let Some(frozen) = frozen else {
            return self.database.transaction(|db| {
                Ok(RetentionReport {
                    reclaimed_sessions: 0,
                    remaining: selection::remaining(db)?,
                })
            });
        };
        let originals = self.database.transaction(|db| {
            if plan::completed(db, &frozen)? {
                return Ok(None);
            }
            if plan::read(db)?.as_ref() != Some(&frozen) {
                return Err(SessionError::CorruptStore);
            }
            plan::check_current(db, &frozen)?;
            frozen
                .proofs()
                .iter()
                .map(|item| proof::load(db, item))
                .collect::<Result<Vec<_>, _>>()
                .map(Some)
        })?;
        let Some(originals) = originals else {
            return self.database.transaction(|db| {
                Ok(RetentionReport {
                    reclaimed_sessions: 0,
                    remaining: selection::remaining(db)?,
                })
            });
        };
        for original in originals {
            let record = self
                .audit
                .append(original.event.clone())
                .map_err(|_| SessionError::AuditUnavailable)?;
            if super::outbox::receipt(&record, &original.event)? != original.receipt {
                return Err(SessionError::CorruptStore);
            }
        }
        let event = frozen.event(&self.database.transaction(schema::store_id)?)?;
        let record = self
            .audit
            .append(event.clone())
            .map_err(|_| SessionError::AuditUnavailable)?;
        let receipt = super::outbox::receipt(&record, &event)?;
        self.database
            .transaction(|db| commit::apply(db, &frozen, &receipt))
    }
}

pub(super) fn epoch_base(db: &Connection) -> Result<i64, SessionError> {
    Ok(checkpoint::read(db)?.epoch_base)
}

pub(super) fn issuance_floor(db: &Connection) -> Result<i64, SessionError> {
    let checkpoint = checkpoint::read(db)?;
    if schema::version(db)? == 1 {
        return Ok(0);
    }
    Ok(plan::read(db)?.map_or(checkpoint.issuance_floor, |plan| {
        checkpoint.issuance_floor.max(plan.issuance_floor)
    }))
}

pub(super) fn event_sequence_barrier(db: &Connection) -> Result<i64, SessionError> {
    Ok(checkpoint::read(db)?.event_sequence_barrier)
}

pub(super) fn genesis() -> Result<String, SessionError> {
    encode(&checkpoint::Checkpoint::genesis())
}

pub(super) fn validate(db: &Connection) -> Result<(), SessionError> {
    checkpoint::validate(db)?;
    if let Some(plan) = plan::read(db)? {
        plan::check_current(db, &plan)?;
    }
    Ok(())
}

fn encode<T: Serialize>(value: &T) -> Result<String, SessionError> {
    serde_json::to_string(value).map_err(|_| SessionError::CorruptStore)
}

fn decode<T: DeserializeOwned + Serialize>(encoded: &str) -> Result<T, SessionError> {
    let value: T = serde_json::from_str(encoded).map_err(|_| SessionError::CorruptStore)?;
    if encode(&value)? != encoded {
        return Err(SessionError::CorruptStore);
    }
    Ok(value)
}

fn digest(bytes: &[u8]) -> String {
    super::secrets::hex(&Sha256::digest(bytes))
}

fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}

fn current_epoch(db: &Connection) -> Result<i64, SessionError> {
    db.query_row("SELECT epoch FROM auth_meta WHERE id=1", [], |row| {
        row.get(0)
    })
    .map_err(unavailable)
}
