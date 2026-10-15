//! 同一对象未发布草案的先后（T-181／BUG-031）：只能发布最新的那一份。
//!
//! 「谁更新」全仓只在这里判：按入库先后，也就是 SQLite 给每行的隐式 rowid。不按提交时间：
//! 时间串由调用方给，时钟会回拨、各端时钟也不一致，jiff 写时间还会去掉小数尾部的 0，
//! 按字符串比在同一秒内会排反。也不按草案号：同一毫秒里生成的 ULID 先后随机。
//! rowid 可靠的前提：本库不 VACUUM，草案行只改状态、不删不换，备份走 SQLite backup 接口按页复制。

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Transaction, params};

use crate::{PolicyError, PolicyKind, database::unavailable, sql};

use super::{DesiredRecord, PolicyStore};

/// 每份未发布草案号 → 同一对象里比它晚入库的未发布草案号（按入库先后，最新那份是空表）。
type NewerOpen = BTreeMap<String, Vec<String>>;

/// 一份未发布草案：所属对象（kind、subject）与草案号。
struct Open {
    kind: String,
    subject: String,
    id: String,
}

/// 未发布草案（pending／validated／rejected）按入库先后排好；给了对象就只取这个对象的。
fn open_in_order(
    tx: &Transaction<'_>,
    object: Option<(PolicyKind, &str)>,
) -> Result<Vec<Open>, PolicyError> {
    let (kind, subject) = match object {
        Some((kind, subject)) => (Some(kind.as_str()), Some(subject)),
        None => (None, None),
    };
    let mut statement = tx
        .prepare(
            "SELECT kind, subject, id FROM policy_desired
             WHERE status IN ('pending', 'validated', 'rejected')
               AND (?1 IS NULL OR (kind = ?1 AND subject = ?2))
             ORDER BY rowid",
        )
        .map_err(unavailable)?;
    let rows = statement
        .query_map(params![kind, subject], |row| {
            Ok(Open {
                kind: row.get(0)?,
                subject: row.get(1)?,
                id: row.get(2)?,
            })
        })
        .map_err(unavailable)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
}

/// 未发布草案的 [`NewerOpen`] 表；给了对象就只算这个对象的。
fn newer_open(
    tx: &Transaction<'_>,
    object: Option<(PolicyKind, &str)>,
) -> Result<NewerOpen, PolicyError> {
    let mut queues: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for open in open_in_order(tx, object)? {
        queues
            .entry((open.kind, open.subject))
            .or_default()
            .push(open.id);
    }
    let mut newer = BTreeMap::new();
    for ids in queues.into_values() {
        for (position, id) in ids.iter().enumerate() {
            newer.insert(id.clone(), ids[position + 1..].to_vec());
        }
    }
    Ok(newer)
}

/// 同一对象里比 `id` 晚入库的未发布草案号（按入库先后）。调用方已在同一事务里确认 `id` 是这个
/// 对象的未发布草案；这里查不到它，只能是行和判定对不上（库被改坏），报库损坏，绝不当成「没有更新的」放行。
pub(super) fn newer_than(
    tx: &Transaction<'_>,
    kind: PolicyKind,
    subject: &str,
    id: &str,
) -> Result<Vec<String>, PolicyError> {
    newer_open(tx, Some((kind, subject)))?
        .remove(id)
        .ok_or(PolicyError::CorruptStore)
}

impl PolicyStore {
    /// desired 列表（同 [`PolicyStore::desired_list`]，不按状态筛），附每份未发布草案「同一对象里
    /// 比它晚入库的未发布草案号」，最新那份是空表。与发布时的拦截同一口径，两者在同一快照内读取。
    /// `limit` 只截已结束的历史：未发布的草案一份不少，落在窗口外的按同一排序补在末尾。界面要靠它
    /// 数全「发布会取代哪几份」，也要找得到拦住发布、该先撤回的那份。
    pub fn desired_queue(
        &self,
        limit: usize,
    ) -> Result<(Vec<DesiredRecord>, NewerOpen), PolicyError> {
        self.db.read(|tx| {
            let mut records = sql::desired_list(tx, None, limit)?;
            let newer = newer_open(tx, None)?;
            let listed: BTreeSet<&str> = records.iter().map(|record| record.id.as_str()).collect();
            let mut outside = newer
                .keys()
                .filter(|id| !listed.contains(id.as_str()))
                .map(|id| sql::desired(tx, id)?.ok_or(PolicyError::CorruptStore))
                .collect::<Result<Vec<_>, _>>()?;
            // 与 desired_list 的 ORDER BY submitted_at DESC, id DESC 同序：SQLite 文本按字节比，String 也是。
            outside.sort_by(|left, right| {
                right
                    .submitted_at
                    .cmp(&left.submitted_at)
                    .then_with(|| right.id.cmp(&left.id))
            });
            records.extend(outside);
            Ok((records, newer))
        })
    }
}
