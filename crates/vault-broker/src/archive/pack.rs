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
use cap_std::fs::Dir;
use std::{
    path::Path,
    sync::{Arc, atomic::Ordering},
};

/// 一步最多看几个请求：不合格的也要读，读多了占着锁。
const EXAMINE: usize = 256;

/// 往前挑的两个目录，按顺序：正文请求（看描述符）、文件操作（看请求）。
const GROUPS: [Part; 2] = [Part::Descriptor, Part::FileRequest];

/// 这一轮用的判据。
#[derive(Clone, Copy)]
struct Rules {
    cutoff: ArchiveCutoff,
    /// 散件到了 95%：已在封存之内的，不满 7 天也打（用户 2026-11-27 定，T-249 G4）。
    urgent: bool,
}
impl Rules {
    /// 年限够了；紧急时不看年限。
    fn old_enough(self, at_ms: Option<i64>) -> bool {
        self.urgent || self.cutoff.aged(at_ms)
    }
}

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
    // 紧不紧急在这一轮开头量一次（数一遍散件），一轮里不变：打着打着降下来了，挑好的照样写。
    let urgent = match cursor.urgent {
        Some(urgent) => urgent,
        None => {
            let urgent = broker.record_usage()?.archive_batch() == Some(ArchiveBatch::All);
            cursor.urgent = Some(urgent);
            urgent
        }
    };
    let rules = Rules { cutoff, urgent };
    let (packed, examined) = if cursor.ready(batch) {
        (write(broker, rules, batch, &cursor.take())?, 0)
    } else {
        (0, scan(broker, rules, cursor)?)
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
    rules: Rules,
    batch: ArchiveBatch,
    held: &[(Part, String)],
) -> Result<usize, BrokerError> {
    let mut packs = Vec::with_capacity(held.len());
    for (group, id) in held {
        packs.extend(candidate(broker, *group, id, rules)?);
    }
    if packs.len() < held.len() && packs.len() < batch.least() {
        return Ok(0);
    }
    publish(broker, &packs)?;
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

/// 从游标往后挑：先正文、后文件操作，各按请求号排。凑满一段或看满一步就停，游标停在最后看过的那个；
/// 交回看了几个。
fn scan(
    broker: &FsVaultBroker,
    rules: Rules,
    cursor: &mut ArchiveCursor,
) -> Result<usize, BrokerError> {
    let mut examined = 0;
    for group in GROUPS {
        // 已经走到文件操作的，正文那一组整个跳过。
        if group == Part::Descriptor && matches!(&cursor.after, Some((Part::FileRequest, _))) {
            continue;
        }
        let path = group.directory();
        for file_name in loose::names(&broker.data, path)? {
            let Some((part, id)) = loose::identify(path, &file_name) else {
                continue;
            };
            if part != group
                || matches!(&cursor.after, Some((seen, after)) if *seen == group && id <= after.as_str())
            {
                continue;
            }
            if examined >= EXAMINE || cursor.full() {
                return Ok(examined);
            }
            if let Some(pack) = candidate(broker, group, id, rules)? {
                let bytes = pack.bytes.iter().map(Vec::len).sum();
                // 这一段装不下：先把装好的写出去，它留到下一段。
                if !cursor.hold(group, id, bytes) {
                    return Ok(examined);
                }
            }
            examined += 1;
            cursor.after = Some((group, id.to_owned()));
        }
    }
    cursor.scanned = true;
    Ok(examined)
}

/// 合格就交回整包的原字节；包的种类由记录本身定。
fn candidate(
    broker: &FsVaultBroker,
    group: Part,
    id: &str,
    rules: Rules,
) -> Result<Option<Pack>, BrokerError> {
    match group {
        Part::Descriptor => content(broker, id, rules),
        Part::FileRequest => file(broker, id, rules),
        _ => Err(BrokerError::CorruptWriterState),
    }
}

/// 一条正文请求：满了年限、有终态、各条都核得过；有审计主体的两份回执都在封存水位内，
/// 没有的（看板投影那种）请求早于最近一次成功封存的内容截止时刻。只看散件，每条只读一遍。
fn content(broker: &FsVaultBroker, id: &str, rules: Rules) -> Result<Option<Pack>, BrokerError> {
    let first = loose::read(&broker.data, Part::Descriptor, id)?
        .ok_or(BrokerError::CaptureChanged)?
        .bytes;
    let envelope = recorded::store::decode_id(broker, id, &first)?;
    if !rules.old_enough(envelope.requested_at_ms) {
        return Ok(None);
    }
    let kind = if envelope.actor.is_some() {
        Kind::Content
    } else {
        Kind::Unaudited
    };
    let Some(pack) = gather(broker, kind, id, &envelope.key_hash, first)? else {
        return Ok(None);
    };
    let replay = pack.bytes.last().ok_or(BrokerError::CorruptWriterState)?;
    let replay = mutation::replay::decode(replay, Some(envelope.pair_id))?;
    let terminal = events::own(&envelope, replay)?;
    recorded::bound(&terminal, &envelope)?;
    if kind == Kind::Unaudited {
        // 没有回执可核：只能按时间判它进没进过封存。
        return Ok(rules
            .cutoff
            .sealed_before(envelope.requested_at_ms)
            .then_some(pack));
    }
    let [_, intent_ack, outcome_ack, _] = pack.bytes.as_slice() else {
        return Err(BrokerError::CorruptWriterState);
    };
    let intent = events::row(&envelope, None)?;
    let outcome = events::row(&envelope, Some(events::outcome(&terminal)))?;
    let offsets = [
        events::decode_ack(broker, &intent, intent_ack)?.offset,
        events::decode_ack(broker, &outcome, outcome_ack)?.offset,
    ];
    Ok(rules.cutoff.sealed(offsets).then_some(pack))
}

/// 一次落定的文件操作：已提交的改名、删除、恢复，取消、拒绝的任何一种。终态未知的不打（可能还要人收口）。
/// 请求满年限；提交了的效果落定也要满年限（请求很早、后来才收口的不算）。两份回执都在封存水位内。
fn file(broker: &FsVaultBroker, id: &str, rules: Rules) -> Result<Option<Pack>, BrokerError> {
    let first = loose::read(&broker.data, Part::FileRequest, id)?
        .ok_or(BrokerError::CaptureChanged)?
        .bytes;
    let request = file_operation::request::decode(broker, id, &first)?;
    if !rules.old_enough(request.frozen.requested_at_ms) {
        return Ok(None);
    }
    let Some(mut pack) = gather(
        broker,
        Kind::FileOperation,
        id,
        &request.frozen.key_hash,
        first,
    )?
    else {
        return Ok(None);
    };
    let [_, intent_ack, outcome_ack, replay] = pack.bytes.as_slice() else {
        return Err(BrokerError::CorruptWriterState);
    };
    let Replay::FileOperation(terminal) =
        mutation::replay::decode(replay, Some(request.frozen.pair_id))?
    else {
        return Err(BrokerError::CorruptWriterState);
    };
    if terminal.frozen != request.frozen {
        return Err(BrokerError::CorruptWriterState);
    }
    let kind = match (terminal.receipt.outcome, &request.frozen.kind) {
        (FileOperationOutcome::Committed, FileOperationKind::Rename { .. }) => Kind::Rename,
        (
            FileOperationOutcome::Committed
            | FileOperationOutcome::Cancelled
            | FileOperationOutcome::Rejected,
            _,
        ) => Kind::FileOperation,
        (FileOperationOutcome::Indeterminate, _) => return Ok(None),
    };
    // 提交了的看效果落定的时刻（一定要有）；取消、拒绝的没有效果就看请求时刻。
    let committed = terminal.receipt.outcome == FileOperationOutcome::Committed;
    if (committed && terminal.effect_at_ms.is_none())
        || !rules.old_enough(terminal.effect_at_ms.or(request.frozen.requested_at_ms))
    {
        return Ok(None);
    }
    let intent = file_operation::audit::intent(&request);
    let outcome = file_operation::audit::outcome(&request, &terminal)?;
    let offsets = [
        file_operation::audit::decode_ack(&intent, intent_ack)?.0,
        file_operation::audit::decode_ack(&outcome, outcome_ack)?.0,
    ];
    pack.kind = kind;
    Ok(rules.cutoff.sealed(offsets).then_some(pack))
}

/// 配齐其余几条散件的原字节；少一条散件就不是一整包，不打。
fn gather(
    broker: &FsVaultBroker,
    kind: Kind,
    id: &str,
    key: &str,
    first: Vec<u8>,
) -> Result<Option<Pack>, BrokerError> {
    let parts = kind.parts();
    let mut pack = Pack {
        kind,
        id: id.to_owned(),
        key: key.to_owned(),
        bytes: Vec::with_capacity(parts.len()),
    };
    pack.bytes.push(first);
    for part in parts.iter().copied().skip(1) {
        let Some(found) = loose::read(&broker.data, part, pack.name_of(part))? else {
            return Ok(None);
        };
        pack.bytes.push(found.bytes);
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
    let existing = match broker.data.symlink_metadata(DIRECTORY) {
        Ok(_) => Some(directory(&broker.data, DIRECTORY)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(BrokerError::io("inspect writer archive", error)),
    };
    // 从这里起要动盘：断在下面哪一步都留着标志，等收口。上面的失败（段数满了、状态读不出来）什么都没动，
    // 不留标志，免得下一轮空收口一次、打一行假的「已收口」。
    broker.archive_dirty.store(true, Ordering::Release);
    let dir = match existing {
        Some(dir) => dir,
        None => create(broker)?,
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
    // 段里有 T-249 才有的种类就是第 10 版：旧程序（包括只认第 9 版的）明确拒绝打开，不会读错。
    let fresh = archive.needs_v10() || packs.iter().any(|pack| pack.kind.needs_v10());
    // 放掉这一份，下面改索引才不用整份复制。
    drop(archive);
    state.archive = Some(addition.head().clone());
    state.format_version = state.format_version.max(if fresh { 10 } else { 9 });
    mutation::save(broker, &state)?;
    let mut archive = broker
        .archive
        .lock()
        .map_err(|_| BrokerError::CorruptWriterState)?;
    Arc::make_mut(&mut archive).apply(addition);
    drop(archive);
    // 已提交：之后断在哪里都只剩删散件。
    mutation::fault(broker, 102)?;
    remove(broker, &originals)?;
    broker.archive_dirty.store(false, Ordering::Release);
    Ok(())
}

/// 第一段：先把目录建好、落盘。
fn create(broker: &FsVaultBroker) -> Result<Dir, BrokerError> {
    broker
        .data
        .create_dir(DIRECTORY)
        .map_err(|e| BrokerError::io("create writer archive", e))?;
    FsVaultBroker::sync_directory(&broker.data, Path::new("."))?;
    directory(&broker.data, DIRECTORY)
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
