//! 生命周期标记保留到 Outcome ACK；终态之后崩溃也能只读派生原 Outcome。

use super::ContentAuditPhase;
pub(crate) use super::event::{
    Ack, Row, digest, name, outcome, outcome_row, own, row, terminal, validate_ack,
};
use crate::history::store;
use crate::mutation::record::Terminal;
use crate::recorded::store::Envelope;
use crate::writer::{directory, disk};
use crate::{BrokerError, FsVaultBroker, records::WriterRecords};

pub(crate) fn entries(records: &dyn WriterRecords) -> Result<Vec<String>, BrokerError> {
    let Some(dir) = store::open(records)? else {
        return Ok(Vec::new());
    };
    let outbox = directory(&dir, "outbox")?;
    let mut names = Vec::new();
    for entry in outbox
        .entries()
        .map_err(|e| BrokerError::io("enumerate content audit", e))?
    {
        if names.len() >= store::MAX_OUTBOX {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let entry = entry.map_err(|e| BrokerError::io("inspect content audit", e))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if !disk::hex(name.strip_suffix("-intent.json").unwrap_or("")) {
            return Err(BrokerError::CorruptWriterState);
        }
        names.push(name);
    }
    names.sort();
    store::open(records)?;
    Ok(names)
}
pub(crate) fn lifecycle(
    broker: &FsVaultBroker,
    filename: &str,
) -> Result<(Envelope, Row), BrokerError> {
    let bytes = store::read(broker, &format!("outbox/{filename}"), disk::RECORD_LIMIT)?
        .ok_or(BrokerError::CorruptWriterState)?;
    let actual: Row = disk::decode(&bytes)?;
    let envelope = crate::recorded::store::load(broker, &actual.recording)?;
    let expected = row(&envelope, None)?;
    if name(&envelope, ContentAuditPhase::Intent) != filename
        || digest(&actual)? != digest(&expected)?
    {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok((envelope, expected))
}
pub(crate) fn acknowledged(
    broker: &FsVaultBroker,
    envelope: &Envelope,
    row: &Row,
) -> Result<bool, BrokerError> {
    Ok(ack(broker, envelope, row)?.is_some())
}
/// 这个阶段的回执，核过是它的；没有就是 `None`。
pub(crate) fn ack(
    records: &dyn WriterRecords,
    envelope: &Envelope,
    row: &Row,
) -> Result<Option<Ack>, BrokerError> {
    let path = format!("requests/{}.ack", name(envelope, row.event.phase));
    store::read(records, &path, disk::RECORD_LIMIT)?
        .map(|bytes| decode_ack(records, row, &bytes))
        .transpose()
}
/// 回执原字节 → 核过的回执。
pub(crate) fn decode_ack(
    records: &dyn WriterRecords,
    row: &Row,
    bytes: &[u8],
) -> Result<Ack, BrokerError> {
    let ack: Ack = disk::decode(bytes)?;
    validate_ack(records, row, &ack)?;
    Ok(ack)
}
pub(crate) fn stage_intent(broker: &FsVaultBroker, envelope: &Envelope) -> Result<(), BrokerError> {
    if envelope.actor.is_none() {
        return Ok(());
    }
    let expected = row(envelope, None)?;
    let filename = name(envelope, ContentAuditPhase::Intent);
    if store::read(broker, &format!("outbox/{filename}"), disk::RECORD_LIMIT)?.is_some() {
        lifecycle(broker, &filename)?;
        return Ok(());
    }
    if let Some(outcome) = outcome_row(broker, envelope)? {
        if acknowledged(broker, envelope, &outcome)? {
            return Ok(());
        }
        return Err(BrokerError::CorruptWriterState);
    }
    if acknowledged(broker, envelope, &expected)? {
        return Err(BrokerError::CorruptWriterState);
    }
    if entries(broker)?.len() >= store::MAX_OUTBOX {
        return Err(BrokerError::WriterLimitExceeded);
    }
    store::save(
        broker,
        &format!("outbox/{filename}"),
        &expected,
        disk::RECORD_LIMIT,
        true,
    )
}
pub(crate) fn stage_outcome(
    broker: &FsVaultBroker,
    terminal: &Terminal,
) -> Result<(), BrokerError> {
    let Some(reference) = &terminal.recording else {
        return Ok(());
    };
    let envelope = crate::recorded::store::load(broker, reference)?;
    if envelope.actor.is_none() {
        return Ok(());
    }
    // 固定终态本身是持久 Outcome 的权威；保留的 Intent 标记让有界扫描发现它。
    let outcome = row(&envelope, Some(outcome(terminal)))?;
    if acknowledged(broker, &envelope, &outcome)? {
        return Ok(());
    }
    lifecycle(broker, &name(&envelope, ContentAuditPhase::Intent))?;
    Ok(())
}
