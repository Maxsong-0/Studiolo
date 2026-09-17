//! 旧完成回执保持原字节，旧 None 只贡献不可清除的全局未知事实。

use super::record::{Outcome, Terminal};
use crate::writer::{directory, disk};
use crate::{BrokerError, MutationReceipt, MutationReplay, VaultUuid};
use cap_std::fs::Dir;

pub(crate) enum Replay {
    Legacy(MutationReplay),
    Current(Terminal),
    FileOperation(Box<crate::file_operation::record::Terminal>),
}

pub(crate) fn decode(bytes: &[u8], pair: Option<VaultUuid>) -> Result<Replay, BrokerError> {
    let value: serde_json::Value = disk::decode(bytes)?;
    if matches!(
        value.get("format_version").and_then(|v| v.as_u64()),
        Some(5 | 6)
    ) {
        let current: crate::file_operation::record::Terminal = disk::decode(bytes)?;
        current.validate(pair.ok_or(BrokerError::CorruptWriterState)?)?;
        Ok(Replay::FileOperation(Box::new(current)))
    } else if value.get("format_version").is_some() {
        let current: Terminal = disk::decode(bytes)?;
        current.validate(pair.ok_or(BrokerError::CorruptWriterState)?)?;
        Ok(Replay::Current(current))
    } else {
        let old: MutationReplay = disk::decode(bytes)?;
        if !disk::hex(&old.fingerprint)
            || old
                .receipt
                .as_ref()
                .is_some_and(|r| r.revision != old.expected_revision || r.path.is_protected())
        {
            return Err(BrokerError::CorruptWriterState);
        }
        Ok(Replay::Legacy(old))
    }
}

pub(crate) fn scan(
    data: &Dir,
    pair: Option<VaultUuid>,
) -> Result<(bool, usize, usize), BrokerError> {
    let dir = directory(data, "idempotency")?;
    let mut count = 0usize;
    let mut total = 0usize;
    let mut unknown = false;
    for entry in dir
        .entries()
        .map_err(|e| BrokerError::io("enumerate mutation records", e))?
    {
        count += 1;
        if count > disk::REPLAY_COUNT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect mutation record", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !disk::hex(name.strip_suffix(".json").unwrap_or("")) {
            return Err(BrokerError::CorruptWriterState);
        }
        let bytes =
            disk::read(&dir, &name, disk::RECORD_LIMIT)?.ok_or(BrokerError::CorruptWriterState)?;
        total = total
            .checked_add(bytes.len())
            .ok_or(BrokerError::WriterLimitExceeded)?;
        if total > disk::REPLAY_BYTES {
            return Err(BrokerError::WriterLimitExceeded);
        }
        if matches!(
            decode(&bytes, pair)?,
            Replay::Legacy(MutationReplay { receipt: None, .. })
        ) {
            unknown = true;
        }
    }
    Ok((unknown, count, total))
}

pub(crate) fn load(data: &Dir, hash: &str, pair: VaultUuid) -> Result<Option<Replay>, BrokerError> {
    if !disk::hex(hash) {
        return Err(BrokerError::CorruptWriterState);
    }
    disk::read(
        data,
        &format!("idempotency/{hash}.json"),
        disk::RECORD_LIMIT,
    )?
    .map(|bytes| decode(&bytes, Some(pair)))
    .transpose()
}

pub(crate) fn matches(
    replay: Replay,
    fingerprint: &str,
    expected: crate::Revision,
    path: &crate::VaultPath,
    precondition: crate::Precondition,
) -> Result<MutationReceipt, BrokerError> {
    let (fp, target) = match &replay {
        Replay::Legacy(r) => (&r.fingerprint, r.expected_revision),
        Replay::Current(r) => (&r.fingerprint, r.expected_revision),
        Replay::FileOperation(_) => return Err(BrokerError::IdempotencyConflict),
    };
    if fp != fingerprint || target != expected {
        return Err(BrokerError::IdempotencyConflict);
    }
    if let Replay::Current(record) = &replay {
        let bound = match &record.outcome {
            Outcome::Committed { .. } => true,
            Outcome::Rejected {
                path: stored,
                precondition: condition,
                ..
            }
            | Outcome::Cancelled {
                path: stored,
                precondition: condition,
            }
            | Outcome::Indeterminate {
                path: stored,
                precondition: condition,
                ..
            } => stored == path && *condition == precondition,
        };
        if !bound {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    let result = match replay {
        Replay::Legacy(r) => r.receipt.ok_or(BrokerError::MutationUnresolved),
        Replay::Current(r) => r.result(),
        Replay::FileOperation(_) => Err(BrokerError::IdempotencyConflict),
    };
    if let Ok(receipt) = &result {
        let previous = match precondition {
            crate::Precondition::MustNotExist => None,
            crate::Precondition::Match(r) => Some(r),
        };
        if &receipt.path != path || receipt.previous_revision != previous {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    result
}
