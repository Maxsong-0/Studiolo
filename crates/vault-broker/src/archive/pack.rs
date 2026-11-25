//! 一步打包：挑合格的请求 → 段文件落盘 → 回读与散件逐字节核 → 存状态（提交点）→ 删散件。
//!
//! 挑和写分在两步：一步往前看若干个、把合格的记在游标里，凑够一段的那一步再逐个重核重读、写出去。
//! 每步都有上界，调用方在步与步之间放锁、看是否在关停。断在任何一处都由 [`super::settle`] 收口：
//! 提交之前的段是没提交的孤儿，散件全在；提交之后没删完的散件和段逐字节相同，接着删。
use super::{
    cutoff::{ArchiveBatch, ArchiveCursor, ArchiveCutoff, ArchiveStep},
    format::{self, DIRECTORY, Kind, MAX_SEGMENTS, Pack, Part},
    index, loose,
};
use crate::{
    BrokerError, FileOperationKind, FileOperationOutcome, FsVaultBroker,
    content_audit::store as events,
    file_operation,
    mutation::{self, replay::Replay},
    recorded,
    records::WriterRecords,
    sha256,
    writer::directory,
};
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

/// 一步最多看几个请求：不合格的也要读，读多了占着锁。
const EXAMINE: usize = 256;

impl FsVaultBroker {
    /// 走一步：往前挑，或者写一段。有未决写入、维护或没投递完的审计事件就不做（`MutationUnresolved`），
    /// 游标留着下一轮接着来；别的出错游标从头再来。
    pub fn archive_step(
        &mut self,
        cutoff: ArchiveCutoff,
        batch: ArchiveBatch,
        cursor: &mut ArchiveCursor,
    ) -> Result<ArchiveStep, BrokerError> {
        idle(self)?;
        let step = advance(self, cutoff, batch, cursor);
        if step.is_err() {
            *cursor = ArchiveCursor::default();
        }
        step
    }
}

fn advance(
    broker: &FsVaultBroker,
    cutoff: ArchiveCutoff,
    batch: ArchiveBatch,
    cursor: &mut ArchiveCursor,
) -> Result<ArchiveStep, BrokerError> {
    if broker.archive_dirty.load(Ordering::Acquire) {
        super::settle(broker)?;
    }
    let (packed, examined) = if cursor.ready(batch) {
        (write(broker, cutoff, batch, &cursor.take())?, 0)
    } else {
        (0, scan(broker, cutoff, cursor)?)
    };
    Ok(ArchiveStep {
        packed,
        examined,
        done: cursor.scanned && !cursor.ready(batch),
    })
}

/// 把挑好的写成一段：逐个重核、重读，上一步之后变了的不打。掉得凑不够了就这一轮不写，下一轮再挑。
fn write(
    broker: &FsVaultBroker,
    cutoff: ArchiveCutoff,
    batch: ArchiveBatch,
    held: &[(Kind, String)],
) -> Result<usize, BrokerError> {
    let mut packs = Vec::with_capacity(held.len());
    for (kind, id) in held {
        packs.extend(candidate(broker, *kind, id, cutoff)?);
    }
    if packs.len() < held.len() && packs.len() < batch.least() {
        return Ok(0);
    }
    broker.archive_dirty.store(true, Ordering::Release);
    publish(broker, &packs)?;
    broker.archive_dirty.store(false, Ordering::Release);
    Ok(packs.len())
}

/// 没有任何在途的事：没有未决写入、批次、文件操作、维护，审计事件也都投递完了。
pub(super) fn idle(broker: &FsVaultBroker) -> Result<(), BrokerError> {
    crate::maintenance::ensure_idle(broker)?;
    if broker.write_status()?.unresolved || !events::entries(broker)?.is_empty() {
        return Err(BrokerError::MutationUnresolved);
    }
    Ok(())
}

/// 从游标往后挑：先正文、后改名，各按请求号排。凑满一段或看满一步就停，游标停在最后看过的那个；
/// 交回看了几个。
fn scan(
    broker: &FsVaultBroker,
    cutoff: ArchiveCutoff,
    cursor: &mut ArchiveCursor,
) -> Result<usize, BrokerError> {
    let mut examined = 0;
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
            if examined >= EXAMINE || cursor.full() {
                return Ok(examined);
            }
            if let Some(pack) = candidate(broker, kind, id, cutoff)? {
                let bytes = pack.bytes.iter().map(Vec::len).sum();
                // 这一段装不下：先把装好的写出去，它留到下一段。
                if !cursor.hold(kind, id, bytes) {
                    return Ok(examined);
                }
            }
            examined += 1;
            cursor.after = Some((kind, id.to_owned()));
        }
    }
    cursor.scanned = true;
    Ok(examined)
}

/// 合格就交回整包的原字节。
fn candidate(
    broker: &FsVaultBroker,
    kind: Kind,
    id: &str,
    cutoff: ArchiveCutoff,
) -> Result<Option<Pack>, BrokerError> {
    match kind {
        Kind::Content => content(broker, id, cutoff),
        Kind::Rename => rename(broker, id, cutoff),
    }
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
    let archive = broker.archive()?;
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
        die(broker, 109);
        broker.writer.check(&broker.root, &broker.data)
    })?;
    // 段已落、状态没存：重开时它是孤儿，散件全在，核过就清掉。
    mutation::fault(broker, 101)?;
    let (length, segment) = index::read_segment(&broker.data, &name, pair, broker.vault_uuid)?;
    let originals = loose::matching(&broker.data, &segment)?.ok_or(BrokerError::CaptureChanged)?;
    let addition = archive.check(name, length, &segment, pair)?;
    // 放掉这一份，下面改索引才不用整份复制。
    drop(archive);
    state.archive = Some(addition.head().clone());
    state.format_version = state.format_version.max(9);
    mutation::save(broker, &state)?;
    let mut archive = broker
        .archive
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    Arc::make_mut(&mut archive).apply(addition);
    drop(archive);
    // 已提交：之后断在哪里都只剩删散件。
    mutation::fault(broker, 102)?;
    remove(broker, &originals)
}

/// 用例用：进程死在这里（恐慌展开，不走出错清理），临时文件原样留在盘上。
fn die(broker: &FsVaultBroker, point: u8) {
    #[cfg(test)]
    if mutation::fault(broker, point).is_err() {
        panic!("进程死在打包的第 {point} 点");
    }
    let _ = (broker, point);
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
