//! 64个、10分钟的有界游标；保留未消费的Broker页，不能丢弃已预取但尚未显示的行。
use super::{seen, wire};
use crate::changes::{PageQuery, check, expired};
use crate::{
    CanonicalWorkbench,
    auth_http::ContentPrincipal,
    coordinator::changes::{ObservationLog, ObservedChange, ObservedKind},
    editing::Problem,
};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use studiolo_core::{VaultPath, VaultUuid};
use studiolo_vault_broker::{ChangeEntry, FsVaultBroker, VaultReader};
#[derive(Clone)]
pub(super) struct Snapshot {
    pub actor: String,
    pub vault: VaultUuid,
    pub project: Option<VaultPath>,
    pub through: u64,
    pub epoch: String,
    pub observation: u64,
    pub generation: u64,
}
#[derive(Clone)]
struct Position {
    snapshot: Snapshot,
    pending: VecDeque<ChangeEntry>,
    next: Option<String>,
    observed_before: u64,
    retention_reported: bool,
    incomplete: bool,
    retired: bool,
    seen: seen::Watermark,
}
#[derive(Default)]
pub(crate) struct Cache {
    entries: VecDeque<(String, Instant, Position)>,
}
impl Cache {
    fn get(&self, key: &str) -> Result<Position, Problem> {
        self.entries
            .iter()
            .find(|(k, at, _)| k == key && at.elapsed() < Duration::from_secs(600))
            .map(|(_, _, p)| p.clone())
            .ok_or_else(expired)
    }
    fn insert(&mut self, position: Position) -> String {
        let key = VaultUuid::new().to_string();
        self.entries
            .retain(|(_, at, _)| at.elapsed() < Duration::from_secs(600));
        while self.entries.len() >= 64 {
            self.entries.pop_front();
        }
        self.entries
            .push_back((key.clone(), Instant::now(), position));
        key
    }
    pub(super) fn snapshot(
        &self,
        key: &str,
        actor: &str,
        vault: VaultUuid,
        project: &Option<VaultPath>,
    ) -> Result<Snapshot, Problem> {
        let s = self.get(key)?.snapshot;
        if s.actor != actor || s.vault != vault || &s.project != project {
            return Err(expired());
        }
        Ok(s)
    }
}
fn includes(project: Option<&VaultPath>, path: &VaultPath) -> bool {
    !path.is_protected()
        && project.is_none_or(|p| {
            path.as_str()
                .strip_prefix(p.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
        })
}
fn observation(position: &mut Position, log: &ObservationLog) -> Option<ObservedChange> {
    if position.observed_before == 0 {
        return None;
    }
    if let Some(row) = log.rows.iter().rev().find(|r| {
        r.index < position.observed_before
            && r.index <= position.snapshot.observation
            && (r
                .path
                .as_ref()
                .is_none_or(|p| includes(position.snapshot.project.as_ref(), p)))
    }) {
        return Some(row.clone());
    }
    if !position.retention_reported && log.lost_through > 0 && position.observed_before > 1 {
        return Some(ObservedChange {
            index: log.lost_through.min(position.observed_before - 1),
            generation: position.snapshot.generation,
            at_ms: None,
            path: None,
            kind: ObservedKind::Gap,
            previous: None,
            revision: None,
            bytes: None,
            gap: Some("retention"),
        });
    }
    position.observed_before = 0;
    None
}
fn refill(
    position: &mut Position,
    broker: &FsVaultBroker,
    calls: &mut usize,
) -> Result<bool, Problem> {
    while position.pending.is_empty() {
        let Some(cursor) = position.next.take() else {
            return Ok(true);
        };
        if *calls >= 2 {
            position.next = Some(cursor);
            return Ok(false);
        }
        *calls += 1;
        let page = broker
            .list_changes(100, Some(&cursor))
            .map_err(crate::history::read_error)?;
        if page.through_sequence != position.snapshot.through {
            return Err(expired());
        }
        position.incomplete |= page.history_incomplete;
        position.retired |= page.retired;
        position.next = page.next_cursor;
        position.pending = page.entries.into();
    }
    Ok(true)
}
/// 四项水位的线格式；列表页和只问水位共用，两边永远是同一个形状。
fn through(sequence: u64, epoch: &str, observation: u64, generation: u64) -> Value {
    json!({"sequence":sequence.to_string(),"epoch":epoch,"observation":observation.to_string(),"generation":generation.to_string()})
}
/// 只回当前水位：和列表第一页签发快照时取的是同一组数，不读任何条目正文、不占游标缓存。
pub(super) fn head(
    owner: &CanonicalWorkbench,
    principal: &ContentPrincipal,
    requested: Option<VaultUuid>,
) -> Result<Value, Problem> {
    let guard = owner
        .core
        .owner
        .lock()
        .map_err(|_| Problem::unavailable())?;
    let broker = guard.as_ref().ok_or_else(Problem::unavailable)?;
    check(owner, principal)?;
    let vault = broker.vault_uuid();
    if requested.is_some_and(|v| v != vault) {
        return Err(Problem::forbidden());
    }
    let sequence = broker
        .list_changes(1, None)
        .map_err(crate::history::read_error)?
        .through_sequence;
    let summary = {
        let log = owner
            .core
            .changes
            .lock()
            .map_err(|_| Problem::unavailable())?;
        through(sequence, &log.epoch, log.last_index, log.generation)
    };
    check(owner, principal)?;
    Ok(json!({"vault_uuid":vault,"through":summary}))
}
/// 第一页的快照：在 owner 锁和观测日志锁里定四项水位、取第一批 Broker 行，放锁之后才读已读水位（T-251）。
///
/// 已读水位要扫审计尾部至多 8192 条、每条本主体的已读 Outcome 再按事件号查一次，用时随账本和已读次数涨；
/// 放在锁外，写入、代次发布、标已读、只问水位都不用等它。之后的取行和续页一样：重新持锁、核快照还属于
/// 本主体本库本范围本 epoch，再按快照固定的上界取，这段空当里新提交的写入和新发布的观察都不会混进这一页。
/// 先定快照、后读水位：已读事实只增不减，读到的水位只会多覆盖、不会少覆盖快照之前落下的已读；
/// 扫描期间才落下的已读如果在本次扫到的范围里就算上，没扫到就显示未读，下次读列表就对了，不会把没读过的标成已读。
fn first(
    owner: &CanonicalWorkbench,
    principal: &ContentPrincipal,
    query: &PageQuery,
    actor: String,
) -> Result<Position, Problem> {
    let mut position = {
        let guard = owner
            .core
            .owner
            .lock()
            .map_err(|_| Problem::unavailable())?;
        let broker = guard.as_ref().ok_or_else(Problem::unavailable)?;
        check(owner, principal)?;
        let vault = broker.vault_uuid();
        if query.vault_uuid.is_some_and(|v| v != vault) {
            return Err(Problem::forbidden());
        }
        let log = owner
            .core
            .changes
            .lock()
            .map_err(|_| Problem::unavailable())?;
        let page = broker
            .list_changes(100, None)
            .map_err(crate::history::read_error)?;
        Position {
            snapshot: Snapshot {
                actor,
                vault,
                project: query.project.clone(),
                through: page.through_sequence,
                epoch: log.epoch.clone(),
                observation: log.last_index,
                generation: log.generation,
            },
            pending: page.entries.into(),
            next: page.next_cursor,
            observed_before: log
                .last_index
                .checked_add(1)
                .ok_or_else(Problem::unavailable)?,
            retention_reported: false,
            incomplete: page.history_incomplete,
            retired: page.retired,
            seen: seen::Watermark::default(),
        }
    };
    position.seen = seen::load(owner.core.content_audit.as_deref(), &position.snapshot);
    Ok(position)
}
pub(super) fn page(
    owner: &CanonicalWorkbench,
    principal: &ContentPrincipal,
    query: PageQuery,
) -> Result<Value, Problem> {
    let actor = crate::changes::actor(principal);
    let fresh = query
        .before
        .is_none()
        .then(|| first(owner, principal, &query, actor.clone()))
        .transpose()?;
    let guard = owner
        .core
        .owner
        .lock()
        .map_err(|_| Problem::unavailable())?;
    let broker = guard.as_ref().ok_or_else(Problem::unavailable)?;
    check(owner, principal)?;
    let vault = broker.vault_uuid();
    if query.vault_uuid.is_some_and(|v| v != vault) {
        return Err(Problem::forbidden());
    }
    let log = owner
        .core
        .changes
        .lock()
        .map_err(|_| Problem::unavailable())?;
    let mut position = match fresh {
        Some(p) => p,
        // 没有刚定的快照就是续页，`before` 一定在。
        None => owner
            .core
            .change_pages
            .lock()
            .map_err(|_| Problem::unavailable())?
            .get(query.before.as_deref().ok_or_else(Problem::unavailable)?)?,
    };
    // 第一页放过一次锁，和续页同样核：库、范围、主体、epoch 有一样变了，这份快照就不能接着用。
    if position.snapshot.actor != actor
        || position.snapshot.vault != vault
        || position.snapshot.project != query.project
        || position.snapshot.epoch != log.epoch
    {
        return Err(expired());
    }
    let mut rows = Vec::new();
    let mut calls = 0;
    let mut scanned = 0;
    while rows.len() < query.limit && scanned < 300 {
        if !refill(&mut position, broker, &mut calls)? {
            break;
        }
        while position
            .pending
            .front()
            .is_some_and(|r| !includes(position.snapshot.project.as_ref(), &r.path))
            && scanned < 300
        {
            position.pending.pop_front();
            scanned += 1;
        }
        if scanned >= 300 {
            break;
        }
        if position.pending.is_empty() && position.next.is_some() {
            continue;
        }
        let observed = observation(&mut position, &log);
        let choose_broker = match (position.pending.front(), &observed) {
            (Some(b), Some(o)) => b.at_ms.unwrap_or(i64::MIN) >= o.at_ms.unwrap_or(i64::MIN),
            (Some(_), None) => true,
            _ => false,
        };
        if choose_broker {
            let row = position.pending.pop_front().expect("刚检查过非空");
            rows.push(wire::broker(&row, position.seen.broker(row.sequence)));
        } else if let Some(row) = observed {
            if row.gap == Some("retention") {
                position.retention_reported = true;
                position.observed_before = 0;
            } else {
                position.observed_before = row.index;
            }
            rows.push(wire::observation(
                &row,
                &position.snapshot.epoch,
                position.seen.observed(&position.snapshot.epoch, row.index),
            ));
        } else {
            break;
        }
        scanned += 1;
    }
    let more = !position.pending.is_empty()
        || position.next.is_some()
        || observation(&mut position, &log).is_some();
    drop(log);
    check(owner, principal)?;
    let summary = through(
        position.snapshot.through,
        &position.snapshot.epoch,
        position.snapshot.observation,
        position.snapshot.generation,
    );
    let incomplete = position.incomplete;
    let retired = position.retired;
    let seen_complete = position.seen.complete;
    let key = owner
        .core
        .change_pages
        .lock()
        .map_err(|_| Problem::unavailable())?
        .insert(position);
    Ok(
        json!({"vault_uuid":vault,"project":query.project,"entries":rows,"next_cursor":more.then_some(&key),"snapshot":key,"through":summary,"history_incomplete":incomplete,"retired":retired,"seen_complete":seen_complete}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn position() -> Position {
        Position {
            snapshot: Snapshot {
                actor: "a".repeat(64),
                vault: VaultUuid::new(),
                project: None,
                through: 0,
                epoch: VaultUuid::new().to_string(),
                observation: 6,
                generation: 3,
            },
            pending: VecDeque::new(),
            next: None,
            observed_before: 7,
            retention_reported: false,
            incomplete: false,
            retired: false,
            seen: seen::Watermark::default(),
        }
    }
    #[test]
    fn expired_evicted_and_cross_scope_cursors_cannot_be_rebound() {
        let mut cache = Cache::default();
        let p = position();
        let first = cache.insert(p.clone());
        assert!(
            cache
                .snapshot(&first, &p.snapshot.actor, p.snapshot.vault, &None)
                .is_ok()
        );
        assert!(
            cache
                .snapshot(&first, "other", p.snapshot.vault, &None)
                .is_err()
        );
        assert!(
            cache
                .snapshot(&first, &p.snapshot.actor, VaultUuid::new(), &None)
                .is_err()
        );
        assert!(
            cache
                .snapshot(
                    &first,
                    &p.snapshot.actor,
                    p.snapshot.vault,
                    &Some(VaultPath::parse("项目/Alpha").unwrap())
                )
                .is_err()
        );
        for _ in 0..64 {
            cache.insert(p.clone());
        }
        assert_eq!(cache.entries.len(), 64);
        assert!(cache.get(&first).is_err());
        let key = cache.insert(p);
        cache.entries.back_mut().unwrap().1 = Instant::now() - Duration::from_secs(601);
        assert!(cache.get(&key).is_err());
    }
    #[test]
    fn evicted_observations_are_one_visible_gap_even_if_eviction_passes_snapshot_upper() {
        let mut p = position();
        let log = ObservationLog {
            lost_through: 50,
            last_index: 60,
            ..Default::default()
        };
        let gap = observation(&mut p, &log).unwrap();
        assert_eq!(gap.index, 6);
        assert_eq!(gap.gap, Some("retention"));
        p.retention_reported = true;
        assert!(observation(&mut p, &log).is_none());
        assert_eq!(p.observed_before, 0);
    }
}
