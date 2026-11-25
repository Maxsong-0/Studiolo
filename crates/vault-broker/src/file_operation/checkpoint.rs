//! 当前活动槽清空后，仍逐项核对永久请求、终态与双 ACK。
use super::{
    audit,
    disk::{self, Folder},
    request, store, terminal,
};
use crate::{
    BrokerError,
    archive::{Archive, Kind},
    checkpoint::{
        audit::{BrokerCheckpointAudit, Verifier},
        check_deadline,
    },
    records::WriterRecords,
    writer::disk as codec,
};
use std::time::Instant;

pub(crate) fn verify(
    source: &dyn WriterRecords,
    deadline: Instant,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    let archive = source.archive()?;
    let packed = || {
        archive
            .packs()
            .iter()
            .filter(|pack| pack.kind == Kind::Rename)
    };
    if store::open(source)?.is_none() {
        // 打过改名包的一定启用过文件操作。
        return if packed().next().is_some() {
            Err(BrokerError::CorruptWriterState)
        } else {
            Ok(())
        };
    }
    let names = loose(source, &archive, Folder::Records)?;
    let acks = loose(source, &archive, Folder::Acks)?;
    let mut expected_acks = Vec::new();
    for name in names {
        check_deadline(deadline)?;
        let id = name
            .strip_suffix(".json")
            .ok_or(BrokerError::CorruptWriterState)?;
        check(source, id, verify)?;
        expected_acks.extend(["intent", "outcome"].map(|phase| format!("{id}-{phase}.json")));
    }
    expected_acks.sort();
    if acks != expected_acks {
        return Err(BrokerError::CorruptWriterState);
    }
    for pack in packed() {
        check_deadline(deadline)?;
        check(source, &pack.id, verify)?;
    }
    store::required(source)?;
    check_deadline(deadline)
}

/// 散件的文件名。打包提交了、散件还没删完的那一刻，两份必须逐字节相同，只算段里那一条。
fn loose(
    source: &dyn WriterRecords,
    archive: &Archive,
    folder: Folder,
) -> Result<Vec<String>, BrokerError> {
    let parent = disk::parent(source, folder)?;
    let mut names = Vec::new();
    for name in disk::names(source, folder)? {
        let (part, id) = folder
            .archived(&name)
            .ok_or(BrokerError::CorruptWriterState)?;
        match archive.read(source.data(), part, id)? {
            None => names.push(name),
            Some(packed) => {
                let kept = codec::read(&parent, &name, super::limits::REQUEST_LIMIT)?;
                if kept.as_ref() != Some(&packed) {
                    return Err(BrokerError::CorruptWriterState);
                }
            }
        }
    }
    Ok(names)
}

/// 一次文件操作：终态在、两份回执都在、账本里有两条原事件。散件和打了包的同一个判法。
fn check(
    source: &dyn WriterRecords,
    id: &str,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    let request = request::load(source, id)?.ok_or(BrokerError::CorruptWriterState)?;
    let terminal =
        terminal::load(source, &request.frozen)?.ok_or(BrokerError::MutationUnresolved)?;
    for event in [
        audit::intent(&request),
        audit::outcome(&request, &terminal)?,
    ] {
        let ack = audit::checkpoint_ack(source, &event)?.ok_or(BrokerError::MutationUnresolved)?;
        verify(BrokerCheckpointAudit::FileOperation(&event), ack.0, &ack.1)?;
    }
    Ok(())
}
