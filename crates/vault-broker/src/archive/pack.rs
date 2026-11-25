//! 一步打包：挑合格的请求 → 段文件落盘 → 回读与散件逐字节核 → 存状态（提交点）→ 删散件。
//!
//! 每步最多 [`SEGMENT_PACKS`] 包，调用方在步与步之间放锁、看是否在关停。断在任何一处都由 [`super::settle`] 收口：
//! 提交之前的段是没提交的孤儿，散件全在；提交之后没删完的散件和段逐字节相同，接着删。
use super::{
    cutoff::{ArchiveCursor, ArchiveCutoff, ArchiveStep},
    format::{self, DIRECTORY, Kind, MAX_SEGMENTS, Pack, Part, SEGMENT_BYTES, SEGMENT_PACKS},
    index, loose,
};
use crate::{
    BrokerError, FileOperationKind, FileOperationOutcome, FsVaultBroker,
    content_audit::store as events,
    file_operation,
    mutation::{self, replay::Replay},
    recorded, sha256,
    writer::directory,
};
use std::{path::Path, sync::atomic::Ordering};

/// 一步最多看几个请求：不合格的也要读，读多了占着锁。
const EXAMINE: usize = 256;
/// 段头最多占这么多；挑的时候先留出来。
const HEADER_BYTES: usize = 256 * 1024;

impl FsVaultBroker {
    /// 打一步包。有未决写入、维护或没投递完的审计事件就不打（`MutationUnresolved`），下一轮再来。
    pub fn archive_step(
        &mut self,
        cutoff: ArchiveCutoff,
        cursor: &mut ArchiveCursor,
    ) -> Result<ArchiveStep, BrokerError> {
        idle(self)?;
        if self.archive_dirty.load(Ordering::Acquire) {
            super::settle(self)?;
        }
        let mut step = ArchiveStep {
            packed: 0,
            examined: 0,
            done: false,
        };
        let packs = select(self, cutoff, cursor, &mut step.examined)?;
        step.done = cursor.done;
        if !packs.is_empty() {
            self.archive_dirty.store(true, Ordering::Release);
            publish(self, &packs)?;
            self.archive_dirty.store(false, Ordering::Release);
            step.packed = packs.len();
        }
        Ok(step)
    }
}

/// 没有任何在途的事：没有未决写入、批次、文件操作、维护，审计事件也都投递完了。
pub(super) fn idle(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    crate::maintenance::ensure_idle(broker)?;
    if broker.write_status()?.unresolved || !events::entries(broker)?.is_empty() {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}

/// 从游标往后挑：先正文、后改名，各按请求号排。挑满一段或看满一步就停，游标停在最后看过的那个。
fn select(
    broker: &FsVaultBroker,
    cutoff: ArchiveCutoff,
    cursor: &mut ArchiveCursor,
    examined: &mut usize,
) -> Result<Vec<Pack>, BrokerError> {
    let mut packs = Vec::new();
    let mut bytes = HEADER_BYTES;
    for kind in [Kind::Content, Kind::Rename] {
        // 已经走到改名的，正文那一类整个跳过。
        if kind == Kind::Content && matches!(&cursor.after, Some((Kind::Rename, _))) {
            continue;
        }
        let path = kind.parts()[0].directory();
        for file_name in loose::names(&broker.data, path)? {
            let Some((part, id)) = loose::identify(path, &file_name) else {
                continue;
            };
            if part != kind.parts()[0]
                || matches!(&cursor.after, Some((seen, after)) if *seen == kind && id <= after.as_str())
            {
                continue;
            }
            if *examined >= EXAMINE || packs.len() >= SEGMENT_PACKS {
                return Ok(packs);
            }
            let candidate = match kind {
                Kind::Content => content(broker, id, cutoff)?,
                Kind::Rename => rename(broker, id, cutoff)?,
            };
            if let Some(pack) = candidate {
                let size: usize = pack.bytes.iter().map(Vec::len).sum();
                if bytes + size > SEGMENT_BYTES {
                    return Ok(packs);
                }
                bytes += size;
                packs.push(pack);
            }
            *examined += 1;
            cursor.after = Some((kind, id.to_owned()));
        }
    }
    cursor.done = true;
    Ok(packs)
}

/// 一条正文请求：两份回执都在且在封存水位内、满了年限、有终态、四条都核得过。只看散件，每条只读一遍。
fn content(
    broker: &FsVaultBroker,
    id: &str,
    cutoff: ArchiveCutoff,
) -> Result<Option<Pack>, BrokerError> {
    let first = loose::read(&broker.data, Part::Descriptor, id)?
        .ok_or(BrokerError::CaptureChanged)?
        .bytes;
    let envelope = recorded::store::decode_id(broker, id, &first)?;
    // 没有审计主体的写入没有回执，按退役条件永远不打包。
    if envelope.actor.is_none() || !cutoff.aged(envelope.requested_at_ms) {
        return Ok(None);
    }
    let Some(pack) = gather(broker, Kind::Content, id, &envelope.key_hash, first)? else {
        return Ok(None);
    };
    let [_, intent_ack, outcome_ack, replay] = &pack.bytes;
    let replay = mutation::replay::decode(replay, Some(envelope.pair_id))?;
    let terminal = events::own(&envelope, replay)?;
    recorded::bound(&terminal, &envelope)?;
    let intent = events::row(&envelope, None)?;
    let outcome = events::row(&envelope, Some(events::outcome(&terminal)))?;
    let offsets = [
        events::decode_ack(broker, &intent, intent_ack)?.offset,
        events::decode_ack(broker, &outcome, outcome_ack)?.offset,
    ];
    Ok(cutoff.sealed(offsets).then_some(pack))
}

/// 一次改名：只认已提交的；删除和恢复不打包。
fn rename(
    broker: &FsVaultBroker,
    id: &str,
    cutoff: ArchiveCutoff,
) -> Result<Option<Pack>, BrokerError> {
    let first = loose::read(&broker.data, Part::FileRequest, id)?
        .ok_or(BrokerError::CaptureChanged)?
        .bytes;
    let request = file_operation::request::decode(broker, id, &first)?;
    if !matches!(request.frozen.kind, FileOperationKind::Rename { .. })
        || !cutoff.aged(request.frozen.requested_at_ms)
    {
        return Ok(None);
    }
    let Some(pack) = gather(broker, Kind::Rename, id, &request.frozen.key_hash, first)? else {
        return Ok(None);
    };
    let [_, intent_ack, outcome_ack, replay] = &pack.bytes;
    let Replay::FileOperation(terminal) =
        mutation::replay::decode(replay, Some(request.frozen.pair_id))?
    else {
        return Err(BrokerError::CorruptWriterState);
    };
    if terminal.frozen != request.frozen {
        return Err(BrokerError::CorruptWriterState);
    }
    // 效果落定的时刻也要满年限：请求很早、后来才收口的不算。
    if terminal.receipt.outcome != FileOperationOutcome::Committed
        || !cutoff.aged(terminal.effect_at_ms)
    {
        return Ok(None);
    }
    let intent = file_operation::audit::intent(&request);
    let outcome = file_operation::audit::outcome(&request, &terminal)?;
    let offsets = [
        file_operation::audit::decode_ack(&intent, intent_ack)?.0,
        file_operation::audit::decode_ack(&outcome, outcome_ack)?.0,
    ];
    Ok(cutoff.sealed(offsets).then_some(pack))
}

/// 配齐另外三条散件的原字节；少一条散件就不是一整包，不打。
fn gather(
    broker: &FsVaultBroker,
    kind: Kind,
    id: &str,
    key: &str,
    first: Vec<u8>,
) -> Result<Option<Pack>, BrokerError> {
    let mut pack = Pack {
        kind,
        id: id.to_owned(),
        key: key.to_owned(),
        bytes: [first, Vec::new(), Vec::new(), Vec::new()],
    };
    for (index, part) in kind.parts().into_iter().enumerate().skip(1) {
        let Some(found) = loose::read(&broker.data, part, pack.name_of(part))? else {
            return Ok(None);
        };
        pack.bytes[index] = found.bytes;
    }
    Ok(Some(pack))
}

fn publish(broker: &FsVaultBroker, packs: &[Pack]) -> Result<(), BrokerError> {
    let pair = broker.writer.binding.pair_id;
    let mut state = mutation::load(broker)?;
    let archive = broker
        .archive
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?
        .as_ref()
        .clone();
    let sequence = archive.head().map_or(1, |head| head.segments + 1);
    if sequence > MAX_SEGMENTS {
        return Err(BrokerError::WriterLimitExceeded);
    }
    let bytes = format::encode(
        &format::Place {
            pair,
            vault: broker.vault_uuid,
            sequence,
            previous: archive.head().map(|head| head.tail.as_str()),
        },
        packs,
    )?;
    let name = format::file_name(sequence, &sha256(&bytes));
    let dir = match broker.data.symlink_metadata(DIRECTORY) {
        Ok(_) => directory(&broker.data, DIRECTORY)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            broker
                .data
                .create_dir(DIRECTORY)
                .map_err(|e| BrokerError::io("create writer archive", e))?;
            FsVaultBroker::sync_directory(&broker.data, Path::new("."))?;
            directory(&broker.data, DIRECTORY)?
        }
        Err(error) => return Err(BrokerError::io("inspect writer archive", error)),
    };
    mutation::fault(broker, 100)?;
    FsVaultBroker::atomic_write_dir_checked(&dir, Path::new(&name), &bytes, true, || {
        broker.writer.check(&broker.root, &broker.data)
    })?;
    // 段已落、状态没存：重开时它是孤儿，散件全在，核过就清掉。
    mutation::fault(broker, 101)?;
    let (length, segment) = index::read_segment(&broker.data, &name, pair, broker.vault_uuid)?;
    let originals = same_as_loose(broker, &segment)?;
    let mut next = archive;
    next.append(name, length, &segment, pair)?;
    state.archive = next.head().cloned();
    state.format_version = state.format_version.max(9);
    mutation::save(broker, &state)?;
    super::install(broker, next)?;
    // 已提交：之后断在哪里都只剩删散件。
    mutation::fault(broker, 102)?;
    remove(broker, &originals)
}

/// 段里的每一条和现在的散件逐字节相同；交回这些散件，提交之后删。
fn same_as_loose(
    broker: &FsVaultBroker,
    segment: &format::Segment,
) -> Result<Vec<loose::Loose>, BrokerError> {
    let mut originals = Vec::with_capacity(segment.packs.len() * 4);
    for (pack, _) in &segment.packs {
        for (part, bytes) in pack.kind.parts().into_iter().zip(&pack.bytes) {
            let found = loose::read(&broker.data, part, pack.name_of(part))?
                .filter(|found| &found.bytes == bytes)
                .ok_or(BrokerError::CaptureChanged)?;
            originals.push(found);
        }
    }
    Ok(originals)
}

fn remove(broker: &FsVaultBroker, originals: &[loose::Loose]) -> Result<(), BrokerError> {
    for (index, original) in originals.iter().enumerate() {
        loose::remove(broker, original)?;
        // 断在一个包删了两条、和删过半的地方。
        match index {
            1 => mutation::fault(broker, 103)?,
            _ if index == originals.len() / 2 => mutation::fault(broker, 104)?,
            _ => {}
        }
    }
    mutation::fault(broker, 105)?;
    let parts: Vec<_> = originals.iter().map(|original| original.part).collect();
    loose::sync(&broker.data, &parts)
}
