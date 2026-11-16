//! 发布成功代次时才记录差分；观察事实不推测作者，内存丢失必须可见。
mod paging;
mod seen;
#[cfg(test)]
mod tests;
mod wire;
use crate::view::Generation;
pub(crate) use paging::Cache;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use studiolo_core::{Revision, VaultPath, VaultUuid};
use studiolo_vault_broker::{ChangeEntry, FsVaultBroker};
pub(crate) const OBSERVATION_LIMIT: usize = 4096;
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Fingerprint {
    pub revision: Revision,
    pub bytes: u64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObservedKind {
    Created,
    Replaced,
    Deleted,
    Gap,
}
#[derive(Clone, Debug)]
pub(crate) struct ObservedChange {
    pub index: u64,
    pub generation: u64,
    pub at_ms: Option<i64>,
    pub path: Option<VaultPath>,
    pub kind: ObservedKind,
    pub previous: Option<Revision>,
    pub revision: Option<Revision>,
    pub bytes: Option<u64>,
    pub gap: Option<&'static str>,
}
pub(crate) struct ObservationLog {
    pub epoch: String,
    pub last_index: u64,
    pub generation: u64,
    pub lost_through: u64,
    pub rows: VecDeque<ObservedChange>,
}
impl Default for ObservationLog {
    fn default() -> Self {
        Self {
            epoch: VaultUuid::new().to_string(),
            last_index: 0,
            generation: 0,
            lost_through: 0,
            rows: VecDeque::new(),
        }
    }
}
impl ObservationLog {
    fn push(&mut self, mut row: ObservedChange) -> Result<(), ()> {
        self.last_index = self.last_index.checked_add(1).ok_or(())?;
        row.index = self.last_index;
        self.rows.push_back(row);
        if self.rows.len() > OBSERVATION_LIMIT {
            let old = self.rows.pop_front().ok_or(())?;
            self.lost_through = old.index;
        }
        Ok(())
    }
    /// 先准备整批再落入环，序号耗尽不能只记录半份代次。
    pub(super) fn publish(
        &mut self,
        previous: Option<&Generation>,
        candidate: &Generation,
        known: &[ChangeEntry],
        explained: bool,
        at_ms: Option<i64>,
    ) -> Result<(), ()> {
        let rows = changes(previous, candidate, known, explained, at_ms);
        self.last_index.checked_add(rows.len() as u64).ok_or(())?;
        for row in rows {
            self.push(row)?;
        }
        self.generation = candidate.number;
        Ok(())
    }
}
fn gap(generation: u64, at_ms: Option<i64>, reason: &'static str) -> ObservedChange {
    ObservedChange {
        index: 0,
        generation,
        at_ms,
        path: None,
        kind: ObservedKind::Gap,
        previous: None,
        revision: None,
        bytes: None,
        gap: Some(reason),
    }
}
fn changes(
    previous: Option<&Generation>,
    candidate: &Generation,
    known: &[ChangeEntry],
    explained: bool,
    at_ms: Option<i64>,
) -> Vec<ObservedChange> {
    let Some(previous) = previous else {
        return vec![gap(candidate.number, at_ms, "startup")];
    };
    let mut rows = Vec::new();
    if !explained {
        rows.push(gap(candidate.number, at_ms, "explanation_incomplete"));
    }
    let paths: BTreeSet<_> = previous
        .revisions
        .keys()
        .chain(candidate.revisions.keys())
        .collect();
    // 只接受本区间的真实提交；匹配结果revision，不凭时钟、文件名或最近任务归属。
    let lower = previous.indexed_through.unwrap_or(0);
    let upper = candidate.indexed_through.unwrap_or(0);
    let explains: BTreeSet<_> = known
        .iter()
        .filter(|r| r.sequence > lower && r.sequence <= upper)
        .map(|r| (&r.path, r.revision))
        .collect();
    for path in paths {
        let before = previous.revisions.get(path);
        let after = candidate.revisions.get(path);
        if before == after || explains.contains(&(path, after.map(|v| v.revision))) {
            continue;
        }
        rows.push(ObservedChange {
            index: 0,
            generation: candidate.number,
            at_ms,
            path: Some(path.clone()),
            kind: match (before, after) {
                (None, Some(_)) => ObservedKind::Created,
                (Some(_), None) => ObservedKind::Deleted,
                _ => ObservedKind::Replaced,
            },
            previous: before.map(|v| v.revision),
            revision: after.map(|v| v.revision),
            bytes: after.map(|v| v.bytes),
            gap: None,
        });
    }
    rows
}
/// 有界解释扫描；任何缺口都保留未知来源，不能阻塞正常内容视图发布。
pub(super) fn known(broker: &FsVaultBroker, lower: u64, upper: u64) -> (Vec<ChangeEntry>, bool) {
    if lower == upper {
        return (Vec::new(), true);
    }
    let mut cursor = None;
    let mut rows = Vec::new();
    let mut complete = true;
    for _ in 0..64 {
        let Ok(page) = broker.list_changes(100, cursor.as_deref()) else {
            return (rows, false);
        };
        if page.through_sequence != upper {
            return (rows, false);
        }
        complete &= !page.history_incomplete && !page.retired;
        let reached = page.entries.iter().any(|r| r.sequence <= lower);
        rows.extend(page.entries.into_iter().filter(|r| r.sequence > lower));
        if rows.len() > OBSERVATION_LIMIT {
            rows.truncate(OBSERVATION_LIMIT);
            return (rows, false);
        }
        if reached || page.next_cursor.is_none() {
            return (rows, complete);
        }
        cursor = page.next_cursor;
    }
    (rows, false)
}
pub(crate) fn now_ms() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|v| i64::try_from(v.as_millis()).ok())
}
pub(crate) type Revisions = BTreeMap<VaultPath, Fingerprint>;

impl crate::CanonicalWorkbench {
    pub(crate) fn check_changes_access(
        &self,
        principal: &crate::auth_http::ContentPrincipal,
    ) -> Result<(), crate::editing::Problem> {
        use crate::editing::Problem;
        principal.verify().map_err(super::content::session_error)?;
        self.core
            .roots
            .revalidate()
            .map_err(|_| Problem::unavailable())?;
        if self
            .core
            .control
            .lock()
            .map_err(|_| Problem::unavailable())?
            .closing
        {
            return Err(Problem::unavailable());
        }
        Ok(())
    }
}

impl crate::CanonicalWorkbench {
    pub(crate) fn changes_page(
        &self,
        principal: &crate::auth_http::ContentPrincipal,
        query: crate::changes::PageQuery,
    ) -> Result<serde_json::Value, crate::editing::Problem> {
        let _operation = principal
            .enter_operation()
            .map_err(super::content::session_error)?;
        paging::page(self, principal, query)
    }
    pub(crate) fn changes_head(
        &self,
        principal: &crate::auth_http::ContentPrincipal,
        vault: Option<VaultUuid>,
    ) -> Result<serde_json::Value, crate::editing::Problem> {
        let _operation = principal
            .enter_operation()
            .map_err(super::content::session_error)?;
        paging::head(self, principal, vault)
    }
    pub(crate) fn mark_changes_seen(
        &self,
        principal: &crate::auth_http::ContentPrincipal,
        input: crate::changes::SeenInput,
    ) -> Result<serde_json::Value, crate::editing::Problem> {
        let _operation = principal
            .enter_operation()
            .map_err(super::content::session_error)?;
        seen::mark(self, principal, input)
    }
}
