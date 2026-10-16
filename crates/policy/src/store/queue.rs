//! 同一对象未发布草案的先后（T-181／BUG-031）。
//!
//! 挡不挡发布、发布取代哪几份、断线重提（T-182）拿回哪一份，这几件事里的「谁更新」全仓只在这里判；
//! 列表的显示顺序不在此列。按入库先后，也就是 SQLite 给每行的隐式 rowid。不按提交时间：
//! 时间串由调用方给，时钟会回拨、各端时钟也不一致，jiff 写时间还会去掉小数尾部的 0，
//! 按字符串比在同一秒内会排反。也不按草案号：同一毫秒里生成的 ULID 先后随机。
//! rowid 可靠的前提：本库不 VACUUM，草案行只改状态、不删不换，备份走 SQLite backup 接口按页复制。
//!
//! 发布规则（用户 2026-10-16 定）：同一提交者（主人，或同一类 AI）有更晚入库的未发布草案，
//! 这份就不能发；不同提交者互不阻挡，AI 交一份新提案挡不住主人。发布只取代同一对象里比它
//! 早入库的未发布草案，比它晚的一律留着待处理，谁的提议都不会被悄悄作废。

use std::collections::{BTreeMap, BTreeSet};

use rusqlite::{Transaction, params};

use crate::{PolicyError, PolicyKind, database::unavailable, sql};

use super::{DesiredRecord, PolicyStore};

/// 每份未发布草案号 → 一串草案号（按入库先后）。
type ByDraft = BTreeMap<String, Vec<String>>;

/// [`PolicyStore::desired_queue`] 的结果：列表、`newer_open`、`blocked_by`。
type DesiredQueue = (Vec<DesiredRecord>, ByDraft, ByDraft);

/// 一份未发布草案：所属对象（kind、subject）、提交者与草案号。
struct Open {
    object: (String, String),
    /// `owner`，或 `agent` 加 AI 类型：同一类 AI 的几个实例算同一提交者。
    submitter: (String, Option<String>),
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
            "SELECT kind, subject, submitter_kind, agent_type, id FROM policy_desired
             WHERE status IN ('pending', 'validated', 'rejected')
               AND (?1 IS NULL OR (kind = ?1 AND subject = ?2))
             ORDER BY rowid",
        )
        .map_err(unavailable)?;
    let rows = statement
        .query_map(params![kind, subject], |row| {
            Ok(Open {
                object: (row.get(0)?, row.get(1)?),
                submitter: (row.get(2)?, row.get(3)?),
                id: row.get(4)?,
            })
        })
        .map_err(unavailable)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(unavailable)
}

/// 挡住 `draft` 发布的：比它晚入库的那几份里，同一提交者交的（按入库先后）。
fn blocking(draft: &Open, later: &[Open]) -> Vec<String> {
    later
        .iter()
        .filter(|other| other.submitter == draft.submitter)
        .map(|other| other.id.clone())
        .collect()
}

/// 每份未发布草案 → 同一对象里比它晚入库的（`newer_open`），以及其中挡住它发布的（`blocked_by`）。
fn order(tx: &Transaction<'_>) -> Result<(ByDraft, ByDraft), PolicyError> {
    let mut objects: BTreeMap<(String, String), Vec<Open>> = BTreeMap::new();
    for open in open_in_order(tx, None)? {
        objects.entry(open.object.clone()).or_default().push(open);
    }
    let mut newer = BTreeMap::new();
    let mut blocked_by = BTreeMap::new();
    for drafts in objects.into_values() {
        for (position, draft) in drafts.iter().enumerate() {
            let later = &drafts[position + 1..];
            newer.insert(
                draft.id.clone(),
                later.iter().map(|other| other.id.clone()).collect(),
            );
            blocked_by.insert(draft.id.clone(), blocking(draft, later));
        }
    }
    Ok((newer, blocked_by))
}

/// 一份要发布的未发布草案在同一对象里的位置。
pub(super) struct Placement {
    /// 挡住它的同一提交者更新草案；非空就不能发布。
    pub(super) blocked_by: Vec<String>,
    /// 比它早入库的未发布草案：发布后标为已取代。
    pub(super) older: Vec<String>,
}

/// `id` 在同一对象未发布草案里的位置。调用方已在同一事务里确认它是这个对象的未发布草案；
/// 这里找不到它，只能是行和判定对不上（库被改坏），报库损坏，绝不当成「没挡、没有较早的」放行。
pub(super) fn place(
    tx: &Transaction<'_>,
    kind: PolicyKind,
    subject: &str,
    id: &str,
) -> Result<Placement, PolicyError> {
    let drafts = open_in_order(tx, Some((kind, subject)))?;
    let position = drafts
        .iter()
        .position(|draft| draft.id == id)
        .ok_or(PolicyError::CorruptStore)?;
    Ok(Placement {
        blocked_by: blocking(&drafts[position], &drafts[position + 1..]),
        older: drafts[..position]
            .iter()
            .map(|draft| draft.id.clone())
            .collect(),
    })
}

/// 某一类 AI 在这个对象上最新的未发布草案号（按入库先后），断线重提拿回的只会是它。它正是发布时
/// 这一提交者没被挡住的那份。主人和别的类型更晚交的不算：重提的结果只取决于提交方自己的草案，
/// 也就不透露别人有没有新草案。
pub(crate) fn latest_open_from_agent(
    tx: &Transaction<'_>,
    agent_type: &str,
    kind: PolicyKind,
    subject: &str,
) -> Result<Option<String>, PolicyError> {
    Ok(open_in_order(tx, Some((kind, subject)))?
        .into_iter()
        .rev()
        .find(|draft| {
            draft.submitter.0 == "agent" && draft.submitter.1.as_deref() == Some(agent_type)
        })
        .map(|draft| draft.id))
}

/// 把这几份较早的未发布草案标为已取代（只在发布事务里调用）。
pub(super) fn supersede(tx: &Transaction<'_>, ids: &[String]) -> Result<(), PolicyError> {
    for id in ids {
        tx.execute(
            "UPDATE policy_desired SET status = 'superseded'
             WHERE id = ?1 AND status IN ('pending', 'validated', 'rejected')",
            params![id],
        )
        .map_err(unavailable)?;
    }
    Ok(())
}

impl PolicyStore {
    /// desired 列表（同 [`PolicyStore::desired_list`]，不按状态筛），附每份未发布草案的先后：
    /// `newer_open` 是同一对象里比它晚入库的未发布草案号（最新那份是空表），`blocked_by` 是其中
    /// 同一提交者的、挡住它发布的那几份。与发布时的拦截同一口径，在同一快照内读取。
    /// `limit` 只截已结束的历史：未发布的草案一份不少，落在窗口外的按同一排序补在末尾。界面要靠它
    /// 数全「发布会取代哪几份」，也要找得到挡住发布、该先撤回的那份。
    pub fn desired_queue(&self, limit: usize) -> Result<DesiredQueue, PolicyError> {
        self.db.read(|tx| {
            let mut records = sql::desired_list(tx, None, limit)?;
            let (newer, blocked_by) = order(tx)?;
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
            Ok((records, newer, blocked_by))
        })
    }
}
