//! 当前活动槽清空后，仍逐项核对永久请求、终态与双 ACK。
use super::{
    audit,
    disk::{self, Folder},
    request, store, terminal,
};
use crate::{
    BrokerError, FsVaultBroker,
    checkpoint::{
        audit::{BrokerCheckpointAudit, Verifier},
        check_deadline,
    },
};
use std::time::Instant;

pub(crate) fn verify(
    broker: &FsVaultBroker,
    deadline: Instant,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    if store::open(broker)?.is_none() {
        return Ok(());
    }
    let records = disk::names(broker, Folder::Records)?;
    let acks = disk::names(broker, Folder::Acks)?;
    let mut expected_acks = Vec::new();
    for name in records {
        check_deadline(deadline)?;
        let id = name
            .strip_suffix(".json")
            .ok_or(BrokerError::CorruptWriterState)?;
        let request = request::load(broker, id)?.ok_or(BrokerError::CorruptWriterState)?;
        let terminal =
            terminal::load(broker, &request.frozen)?.ok_or(BrokerError::MutationUnresolved)?;
        for event in [
            audit::intent(&request),
            audit::outcome(&request, &terminal)?,
        ] {
            let ack =
                audit::checkpoint_ack(broker, &event)?.ok_or(BrokerError::MutationUnresolved)?;
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
    store::required(broker)?;
    check_deadline(deadline)
}
