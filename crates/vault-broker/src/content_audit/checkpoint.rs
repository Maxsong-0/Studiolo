//! 扫描永久描述符与双 ACK，不以已清空的 outbox 代替历史回执覆盖。
use super::{event, store};
use crate::{
    BrokerError, FsVaultBroker,
    checkpoint::{
        audit::{BrokerCheckpointAudit, Verifier},
        check_deadline,
    },
    history::store as history,
    recorded,
    writer::{directory, disk},
};
use std::time::Instant;

pub(crate) fn verify(
    broker: &FsVaultBroker,
    deadline: Instant,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    if !store::entries(broker)?.is_empty() {
        return Err(BrokerError::MutationUnresolved);
    }
    let Some(root) = history::open(broker, false)? else {
        return Ok(());
    };
    let requests = directory(&root, "requests")?;
    for (index, entry) in requests
        .entries()
        .map_err(|e| BrokerError::io("枚举封存正文回执", e))?
        .enumerate()
    {
        check_deadline(deadline)?;
        if index >= 3 * disk::REPLAY_COUNT {
            return Err(BrokerError::WriterLimitExceeded);
        }
        let name = entry
            .map_err(|e| BrokerError::io("读取封存正文回执名", e))?
            .file_name()
            .into_string()
            .map_err(|_| BrokerError::CorruptWriterState)?;
        if let Some((id, phase)) = ack_name(&name) {
            // 同时核对孤儿 ACK；不能只枚举描述符后漏掉多余或已失去母本的回执。
            let envelope =
                recorded::store::load_id(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
            let row = match phase {
                crate::ContentAuditPhase::Intent => event::row(&envelope, None)?,
                crate::ContentAuditPhase::Outcome => {
                    event::outcome_row(broker, &envelope)?.ok_or(BrokerError::MutationUnresolved)?
                }
            };
            let ack = read_ack(broker, &envelope, &row)?;
            verify(
                BrokerCheckpointAudit::Content(&row.event),
                ack.offset,
                &ack.record_hash,
            )?;
        } else {
            let id = name
                .strip_suffix(".json")
                .filter(|id| disk::hex(id))
                .ok_or(BrokerError::CorruptWriterState)?;
            let envelope =
                recorded::store::load_id(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
            if envelope.actor.is_some() {
                let intent = event::row(&envelope, None)?;
                read_ack(broker, &envelope, &intent)?;
                let outcome = event::outcome_row(broker, &envelope)?
                    .ok_or(BrokerError::MutationUnresolved)?;
                read_ack(broker, &envelope, &outcome)?;
            }
        }
    }
    history::open(broker, false)?;
    check_deadline(deadline)
}
fn ack_name(name: &str) -> Option<(&str, crate::ContentAuditPhase)> {
    name.strip_suffix("-intent.json.ack")
        .map(|id| (id, crate::ContentAuditPhase::Intent))
        .or_else(|| {
            name.strip_suffix("-outcome.json.ack")
                .map(|id| (id, crate::ContentAuditPhase::Outcome))
        })
}
fn read_ack(
    broker: &FsVaultBroker,
    envelope: &recorded::store::Envelope,
    row: &event::Row,
) -> Result<event::Ack, BrokerError> {
    let path = format!("requests/{}.ack", event::name(envelope, row.event.phase));
    let bytes =
        history::read(broker, &path, disk::RECORD_LIMIT)?.ok_or(BrokerError::MutationUnresolved)?;
    let ack: event::Ack = disk::decode(&bytes)?;
    event::validate_ack(broker, row, &ack)?;
    Ok(ack)
}
