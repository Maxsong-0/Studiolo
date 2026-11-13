use super::super::{
    catalog::{self, Item},
    store,
};
use crate::{BrokerError, FsVaultBroker, VaultPath, VaultUuid, sha256};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
pub(super) const SLOTS: u64 = 16;
pub(super) const ROWS: usize = 512;
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Row {
    pub path: VaultPath,
    pub sequence: u64,
    pub id: String,
    pub retired: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Bucket {
    pub version: u32,
    pub pair: VaultUuid,
    pub number: u64,
    pub rows: Vec<Row>,
    #[serde(skip)]
    original: Option<String>,
}
fn name(number: u64) -> String {
    format!("catalogs/feed-v1-{number}.json")
}
pub(in crate::history) fn number(sequence: u64) -> u64 {
    sequence.saturating_sub(1) / SLOTS
}
pub(super) fn load(broker: &FsVaultBroker, number: u64) -> Result<Option<Bucket>, BrokerError> {
    let Some(bytes) = store::read(broker, &name(number), store::CATALOG_BYTES)? else {
        return Ok(None);
    };
    let mut value: Bucket = crate::writer::disk::decode(&bytes)?;
    if value.version != 1
        || value.pair != broker.writer.binding.pair_id
        || value.number != number
        || value.rows.len() > ROWS
    {
        return Err(BrokerError::CorruptWriterState);
    }
    let allocated = crate::mutation::load(broker)?.next_sequence;
    let mut slots = BTreeSet::new();
    for row in &value.rows {
        if row.path.is_protected()
            || row.sequence == 0
            || row.sequence >= allocated
            || self::number(row.sequence) != number
            || !crate::writer::disk::hex(&row.id)
            || !slots.insert((row.sequence, row.path.clone()))
        {
            return Err(BrokerError::CorruptWriterState);
        }
    }
    if value.rows.windows(2).any(|r| {
        (std::cmp::Reverse(r[0].sequence), &r[0].path)
            >= (std::cmp::Reverse(r[1].sequence), &r[1].path)
    }) {
        return Err(BrokerError::CorruptWriterState);
    }
    value.original = Some(sha256(&bytes));
    Ok(Some(value))
}
fn save(broker: &FsVaultBroker, bucket: &Bucket) -> Result<(), BrokerError> {
    if bucket.rows.len() > ROWS {
        return Err(BrokerError::WriterLimitExceeded);
    }
    store::save_catalog(
        broker,
        &name(bucket.number),
        bucket,
        bucket.original.as_deref(),
    )
}
pub(in crate::history) fn record(
    broker: &FsVaultBroker,
    path: &VaultPath,
    item: &Item,
) -> Result<(), BrokerError> {
    super::super::evidence::validate(broker, path, item)?;
    let number = number(item.sequence);
    let mut bucket = load(broker, number)?.unwrap_or(Bucket {
        version: 1,
        pair: broker.writer.binding.pair_id,
        number,
        rows: Vec::new(),
        original: None,
    });
    if let Some(row) = bucket
        .rows
        .iter_mut()
        .find(|r| r.sequence == item.sequence && &r.path == path)
    {
        if row.id == item.entry.id && !row.retired {
            return Ok(());
        }
        // 未公开的原提交已整体回滚，同槽只能留下最终回滚，不能事后冒充成功提交。
        if row.retired
            || super::super::batch::visible(
                broker,
                &self::item(broker, row)?.ok_or(BrokerError::CorruptWriterState)?,
            )?
        {
            return Err(BrokerError::CorruptWriterState);
        }
        row.id = item.entry.id.clone();
    } else {
        bucket.rows.push(Row {
            path: path.clone(),
            sequence: item.sequence,
            id: item.entry.id.clone(),
            retired: false,
        });
    }
    bucket.rows.sort_by(|a, b| {
        b.sequence
            .cmp(&a.sequence)
            .then_with(|| a.path.cmp(&b.path))
    });
    save(broker, &bucket)?;
    crate::mutation::fault(broker, 90)
}
pub(in crate::history) fn retire(
    broker: &FsVaultBroker,
    path: &VaultPath,
    entries: &[(u64, String)],
) -> Result<(), BrokerError> {
    let numbers: BTreeSet<_> = entries
        .iter()
        .filter(|(s, _)| *s > 0)
        .map(|(s, _)| number(*s))
        .collect();
    for number in numbers {
        let Some(mut bucket) = load(broker, number)? else {
            continue;
        };
        let mut changed = false;
        for row in &mut bucket.rows {
            if &row.path == path
                && entries
                    .iter()
                    .any(|(s, id)| *s == row.sequence && id == &row.id)
                && !row.retired
            {
                row.retired = true;
                changed = true;
            }
        }
        if changed {
            save(broker, &bucket)?;
        }
    }
    Ok(())
}
pub(super) fn item(broker: &FsVaultBroker, row: &Row) -> Result<Option<Item>, BrokerError> {
    let catalog = catalog::load(broker, &row.path)?;
    let found = catalog.entries.iter().find(|i| i.entry.id == row.id);
    if row.retired {
        if found.is_some() {
            return Err(BrokerError::CorruptWriterState);
        }
        return Ok(None);
    }
    let Some(found) = found else {
        if catalog
            .feed_retire
            .iter()
            .any(|(s, id)| *s == row.sequence && id == &row.id)
        {
            return Ok(None);
        }
        return Err(BrokerError::CorruptWriterState);
    };
    if found.sequence != row.sequence {
        return Err(BrokerError::CorruptWriterState);
    }
    Ok(Some(found.clone()))
}
