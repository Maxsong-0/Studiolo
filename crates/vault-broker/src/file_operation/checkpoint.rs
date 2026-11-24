//! 当前活动槽清空后，仍逐项核对永久请求、终态与双 ACK。
use super::{
    audit,
    disk::{self, Folder},
    request, store, terminal,
};
use crate::{
    BrokerError,
    checkpoint::{
        audit::{BrokerCheckpointAudit, Verifier},
        check_deadline,
    },
    records::WriterRecords,
};
use std::time::Instant;

pub(crate) fn verify(
    source: &dyn WriterRecords,
    deadline: Instant,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    if store::open(source)?.is_none() {
        return Ok(());
    }
    let names = disk::names(source, Folder::Records)?;
    let acks = disk::names(source, Folder::Acks)?;
    let mut expected_acks = Vec::new();
    for name in names {
        check_deadline(deadline)?;
        let id = name
            .strip_suffix(".json")
            .ok_or(BrokerError::CorruptWriterState)?;
        let request = request::load(source, id)?.ok_or(BrokerError::CorruptWriterState)?;
        let terminal =
            terminal::load(source, &request.frozen)?.ok_or(BrokerError::MutationUnresolved)?;
        for event in [
            audit::intent(&request),
            audit::outcome(&request, &terminal)?,
        ] {
            let ack =
                audit::checkpoint_ack(source, &event)?.ok_or(BrokerError::MutationUnresolved)?;
            verify(BrokerCheckpointAudit::FileOperation(&event), ack.0, &ack.1)?;
            let phase = match event.phase {
                crate::ContentAuditPhase::Intent => "intent",
                crate::ContentAuditPhase::Outcome => "outcome",
            };
            expected_acks.push(format!("{id}-{phase}.json"));
        }
    }
    expected_acks.sort();
    if acks != expected_acks {
        return Err(BrokerError::CorruptWriterState);
    }
    store::required(source)?;
    check_deadline(deadline)
}
