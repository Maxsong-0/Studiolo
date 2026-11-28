//! 扫描永久描述符与双 ACK，不以已清空的 outbox 代替历史回执覆盖。
use super::{event, store};
use crate::{
    BrokerError,
    archive::{Kind, Part, content_part},
    checkpoint::{
        audit::{BrokerCheckpointAudit, Verifier},
        check_deadline,
    },
    history::store as history,
    recorded,
    records::WriterRecords,
    writer::{directory, disk},
};
use std::time::Instant;

pub(crate) fn verify(
    records: &dyn WriterRecords,
    deadline: Instant,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    if !store::entries(records)?.is_empty() {
        return Err(BrokerError::MutationUnresolved);
    }
    let archive = records.archive()?;
    let Some(root) = history::open(records)? else {
        // 打过正文包的一定有历史元数据目录；目录没了不能当成「没有审计历史」。
        return if archive.packs().iter().any(|pack| pack.kind.content()) {
            Err(BrokerError::CorruptWriterState)
        } else {
            Ok(())
        };
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
        let (part, id) = content_part(&name).ok_or(BrokerError::CorruptWriterState)?;
        // 打包提交了、散件还没删完的那一刻：两份必须逐字节相同，只算一条，在下面核。
        if let Some(packed) = archive.read(records.data(), part, id)? {
            if disk::read(&requests, &name, disk::RECORD_LIMIT)?.as_ref() != Some(&packed) {
                return Err(BrokerError::CorruptWriterState);
            }
            continue;
        }
        check(records, part, id, verify)?;
    }
    for pack in archive.packs() {
        check_deadline(deadline)?;
        match pack.kind {
            Kind::Content => {
                for part in [Part::IntentAck, Part::OutcomeAck, Part::Descriptor] {
                    check(records, part, &pack.id, verify)?;
                }
            }
            // 没有审计主体的：只有描述符，没有回执可核（和散件一样，见 `check`）。
            Kind::Unaudited => check(records, Part::Descriptor, &pack.id, verify)?,
            Kind::Rename | Kind::FileOperation => {}
        }
    }
    history::open(records)?;
    check_deadline(deadline)
}

/// 核一条：回执要有母本、和原事件对得上、账本里有原事件；描述符要有两份回执。散件和打了包的同一个判法。
fn check(
    records: &dyn WriterRecords,
    part: Part,
    id: &str,
    verify: &mut Verifier<'_>,
) -> Result<(), BrokerError> {
    // 同时核对孤儿 ACK；不能只枚举描述符后漏掉多余或已失去母本的回执。
    let envelope = recorded::store::load_id(records, id)?.ok_or(BrokerError::CorruptWriterState)?;
    let outcome = || event::outcome_row(records, &envelope)?.ok_or(BrokerError::MutationUnresolved);
    match part {
        Part::IntentAck | Part::OutcomeAck => {
            let row = if part == Part::IntentAck {
                event::row(&envelope, None)?
            } else {
                outcome()?
            };
            let ack = read_ack(records, &envelope, &row)?;
            verify(
                BrokerCheckpointAudit::Content(&row.event),
                ack.offset,
                &ack.record_hash,
            )
        }
        _ => {
            if envelope.actor.is_some() {
                read_ack(records, &envelope, &event::row(&envelope, None)?)?;
                read_ack(records, &envelope, &outcome()?)?;
            }
            Ok(())
        }
    }
}
fn read_ack(
    records: &dyn WriterRecords,
    envelope: &recorded::store::Envelope,
    row: &event::Row,
) -> Result<event::Ack, BrokerError> {
    store::ack(records, envelope, row)?.ok_or(BrokerError::MutationUnresolved)
}
