//! T-130 跨项目只读汇总：一次取全未结束工作项与时间窗内的任务事件，供驾驶舱与协作拓扑使用。
//!
//! 与 `/api/v1/board` 同一会话门禁、同一许可与同一 effective 策略哈希；只读，不写审计。
//! 响应只含元数据：不含指令、验收标准、交接与回执正文、产物指针或审计 payload。
//! 契约全文见 `docs/contracts/board-overview.md`。
use super::BoardPlane;
use axum::{
    Extension, Json,
    extract::{Query, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap, hash_map::Entry},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
use studiolo_audit_ledger::{AuditFilter, AuditLedger, AuditPhase, AuditRecord};
use studiolo_control_db::{AttemptPhase, TaskStatus};
use studiolo_core::{AttemptId, TaskId, VaultPath};
use studiolo_task_engine::{BoardView, ReceiptStatus};

const DEFAULT_WINDOW_HOURS: u32 = 24;
const MAX_WINDOW_HOURS: u32 = 168;

/// 单次汇总的全部上限。生产只用 [`LIMITS`]；单测换小上限覆盖截断与扫满分支。
#[derive(Clone, Copy)]
struct Limits {
    /// 响应最多的工作项条数。
    items: usize,
    /// 响应最多的事件条数，保留最新的。
    events: usize,
    /// 每页审计记录数。
    page: u64,
    /// 单次请求最多倒扫的审计记录数；扫满仍未越过窗口起点即判「不完整」。
    scan: u64,
}

const LIMITS: Limits = Limits {
    items: 500,
    events: 1000,
    page: 4096,
    scan: 65_536,
};

/// 停扫判据额外留出的时钟回拨余量：整页都早于「窗口起点减此余量」才认定再往前只会更早。
const CLOCK_SLACK: SignedDuration = SignedDuration::from_mins(10);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct OverviewQuery {
    window_hours: Option<u32>,
}

pub(super) async fn overview(
    plane: Option<Extension<Arc<BoardPlane>>>,
    query: Result<Query<OverviewQuery>, QueryRejection>,
) -> Response {
    // 反序列化失败与窗口越界都是畸形查询串：同一固定中文 JSON，不回显字段名（与 T-113 横向一致）。
    let invalid = || crate::invalid_query("invalid_board_overview_query");
    // 查询串拒绝仍先于平面检查，与 `/api/v1/board` 的顺序一致。
    let Ok(Query(query)) = query else {
        return invalid();
    };
    let Some(Extension(plane)) = plane else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let hours = query.window_hours.unwrap_or(DEFAULT_WINDOW_HOURS);
    if !(1..=MAX_WINDOW_HOURS).contains(&hours) {
        return invalid();
    }
    let permit = match plane.permits.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => return StatusCode::TOO_MANY_REQUESTS.into_response(),
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        summarize(&plane, hours, LIMITS)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

#[derive(Serialize)]
struct Overview {
    generated_at: String,
    window_from: String,
    window_to: String,
    policy_hash: String,
    items: Vec<Item>,
    items_truncated: bool,
    events: Vec<Event>,
    events_truncated: bool,
}

#[derive(Serialize)]
struct Item {
    id: TaskId,
    title: String,
    project: String,
    status: TaskStatus,
    parent: Option<TaskId>,
    adapter: String,
    holder: Option<String>,
    holder_lineage: Vec<LineageNode>,
    paths: Vec<VaultPath>,
    attempt: Option<Attempt>,
    progress: Option<Progress>,
    receipt_status: Option<ReceiptStatus>,
}

#[derive(Clone, Serialize)]
struct LineageNode {
    id: String,
    agent_type: String,
}

#[derive(Serialize)]
struct Attempt {
    phase: AttemptPhase,
    worker: String,
    active: bool,
    lease_until_ms: i64,
}

#[derive(Serialize)]
struct Progress {
    tools: u64,
    edits: u64,
    last_progress_ms: i64,
}

#[derive(Serialize)]
struct Event {
    at: String,
    project: String,
    task_id: TaskId,
    action: String,
    phase: AuditPhase,
    attempt_id: Option<AttemptId>,
}

fn summarize(plane: &BoardPlane, hours: u32, limits: Limits) -> Result<Overview, String> {
    plane.storage.check().map_err(str::to_owned)?;
    let policy_hash = plane
        .policy
        .effective()
        .map_err(|_| "策略不可用")?
        .effective_hash;
    let to = Timestamp::now();
    let from = to
        .checked_sub(SignedDuration::from_hours(i64::from(hours)))
        .map_err(|_| "时间窗无效")?;
    // 只在读工作项时持有引擎锁；审计倒扫与实例链查询不阻塞黑板写入。
    let views = plane
        .engine
        .lock()
        .map_err(|_| "黑板不可用")?
        .board_list_all(&policy_hash)
        .map_err(|e| e.to_string())?;
    let projects = views
        .iter()
        .map(|view| (view.task.task_id, view.task.spec.project.as_str()))
        .collect::<HashMap<_, _>>();
    let scanned = scan(
        plane.audit.as_ref(),
        &plane.overview_tail,
        &projects,
        (from, to),
        limits,
    )?;
    drop(projects);
    let selection = select(views, scanned, limits);
    let mut chains: HashMap<String, Vec<LineageNode>> = HashMap::new();
    let mut items = Vec::with_capacity(selection.views.len());
    for view in selection.views {
        let lineage = match &view.holder {
            None => Vec::new(),
            Some(holder) => match chains.entry(holder.clone()) {
                Entry::Occupied(known) => known.get().clone(),
                Entry::Vacant(slot) => {
                    // 与 `/api/v1/board` 的 instance_lineages 同源：实例库给出叶→根，这里翻成根→叶。
                    let chain = plane
                        .identities
                        .instance_lineage(holder)
                        .map_err(|_| "实例链不可用")?;
                    slot.insert(
                        chain
                            .iter()
                            .rev()
                            .map(|instance| LineageNode {
                                id: instance.id.clone(),
                                agent_type: instance.agent_type.clone(),
                            })
                            .collect(),
                    )
                    .clone()
                }
            },
        };
        items.push(item(view, lineage)?);
    }
    Ok(Overview {
        generated_at: to.to_string(),
        window_from: from.to_string(),
        window_to: to.to_string(),
        policy_hash,
        items,
        items_truncated: selection.items_truncated,
        events: selection
            .events
            .into_iter()
            .map(|found| Event {
                at: found.at.to_string(),
                project: found.project,
                task_id: found.task_id,
                action: found.action,
                phase: found.phase,
                attempt_id: found.attempt_id,
            })
            .collect(),
        events_truncated: selection.events_truncated,
    })
}

/// 窗口内的一条任务事件；`offset` 只用于同一时刻的稳定排序，不进响应。
struct Found {
    offset: u64,
    at: Timestamp,
    project: String,
    task_id: TaskId,
    action: String,
    phase: AuditPhase,
    attempt_id: Option<AttemptId>,
}

struct Scan {
    /// 已扫范围内落在窗口里的全部黑板任务事件，未排序、未截断。
    events: Vec<Found>,
    /// 已确认覆盖整个窗口；扫满上限仍未越过窗口起点时为 false。
    complete: bool,
}

/// 审计账本没有时间或任务索引，只能按偏移翻页：先定位尾部，再从尾部整页倒扫，
/// 直到某页整页早于「窗口起点减时钟余量」、扫到账本开头，或扫满上限。
/// 追加在写事务内串行取号并打时间戳，时间随偏移单调不减；余量只兜底小幅时钟回拨。
fn scan(
    audit: &dyn AuditLedger,
    tail_hint: &AtomicU64,
    projects: &HashMap<TaskId, &str>,
    (from, to): (Timestamp, Timestamp),
    limits: Limits,
) -> Result<Scan, String> {
    let horizon = from.checked_sub(CLOCK_SLACK).map_err(|_| "时间窗无效")?;
    let floor = tail_floor(audit, tail_hint.load(Ordering::Relaxed), limits.page)?;
    let tail = page(audit, floor, limits.page)?;
    let newest = floor.checked_add(tail.len() as u64).ok_or("审计偏移溢出")?;
    tail_hint.fetch_max(newest, Ordering::Relaxed);
    let mut events = Vec::new();
    let mut scanned = tail.len() as u64;
    let mut older = collect(&tail, projects, (from, to), horizon, &mut events);
    let mut upper = floor;
    let complete = loop {
        if older || upper == 0 {
            break true;
        }
        if scanned >= limits.scan {
            break false;
        }
        let lower = upper.saturating_sub(limits.page);
        let records = page(audit, lower, upper - lower)?;
        if records.len() as u64 != upper - lower {
            return Err("审计账本不连续".into());
        }
        scanned += upper - lower;
        older = collect(&records, projects, (from, to), horizon, &mut events);
        upper = lower;
    };
    Ok(Scan { events, complete })
}

/// 读偏移 `(after, after + count]` 并核对连续；尾页可能不足 `count` 条。
fn page(audit: &dyn AuditLedger, after: u64, count: u64) -> Result<Vec<AuditRecord>, String> {
    let limit = usize::try_from(count).map_err(|_| "审计分页过大")?;
    let records = audit
        .query(&AuditFilter::default().after_offset(after).with_limit(limit))
        .map_err(|_| "审计不可用")?;
    if records
        .iter()
        .zip(1u64..)
        .any(|(record, index)| after.checked_add(index) != Some(record.offset()))
    {
        return Err("审计账本不连续".into());
    }
    Ok(records)
}

/// 找 `floor`，使账本末条偏移 L 满足 `floor <= L < floor + page`：先核实上次记下的尾部
/// 提示仍在账本内（账本被换成更短的也能回退到 0），再倍增步长向后探、二分收窄。
/// 每次探测只取一条记录，稳态下提示有效时只需两次探测。
fn tail_floor(audit: &dyn AuditLedger, hint: u64, page: u64) -> Result<u64, String> {
    let mut low = if present(audit, hint)? { hint } else { 0 };
    let mut step = page;
    let mut high = loop {
        let probe = low.checked_add(step).ok_or("审计偏移溢出")?;
        if !present(audit, probe)? {
            break probe;
        }
        low = probe;
        step = step.saturating_mul(2);
    };
    while high - low > page {
        let middle = low + (high - low) / 2;
        if present(audit, middle)? {
            low = middle;
        } else {
            high = middle;
        }
    }
    Ok(low)
}

/// 偏移 `offset` 处是否已有记录；0 是空前缀，恒在。
fn present(audit: &dyn AuditLedger, offset: u64) -> Result<bool, String> {
    let Some(before) = offset.checked_sub(1) else {
        return Ok(true);
    };
    let records = audit
        .query(&AuditFilter::default().after_offset(before).with_limit(1))
        .map_err(|_| "审计不可用")?;
    match records.first() {
        None => Ok(false),
        Some(record) if record.offset() == offset => Ok(true),
        Some(_) => Err("审计账本不连续".into()),
    }
}

/// 收下一页里落在窗口内、主体是黑板任务的事件（与项目回放同一来源）；
/// 整页都早于 `horizon` 时返回 true，表示再往前只会更早。
fn collect(
    records: &[AuditRecord],
    projects: &HashMap<TaskId, &str>,
    (from, to): (Timestamp, Timestamp),
    horizon: Timestamp,
    events: &mut Vec<Found>,
) -> bool {
    for record in records {
        let at = record.recorded_at();
        if at < from || at > to {
            continue;
        }
        let event = record.event();
        let Some(task_id) = event.subject().task_id() else {
            continue;
        };
        let Some(project) = projects.get(&task_id) else {
            continue;
        };
        events.push(Found {
            offset: record.offset(),
            at,
            project: (*project).to_owned(),
            task_id,
            action: event.action().to_owned(),
            phase: event.phase(),
            attempt_id: event.subject().attempt_id(),
        });
    }
    !records.is_empty() && records.iter().all(|record| record.recorded_at() < horizon)
}

struct Selection {
    views: Vec<BoardView>,
    items_truncated: bool,
    events: Vec<Found>,
    events_truncated: bool,
}

/// 选项、排序与截断：未结束项全收，已结束项须在窗口内有事件；工作项按 (项目, id)、
/// 事件按 (时间, 偏移) 升序。事件超限保留最新的；未扫完窗口时两类都标截断。
fn select(mut views: Vec<BoardView>, scanned: Scan, limits: Limits) -> Selection {
    let Scan {
        mut events,
        complete,
    } = scanned;
    let recent = events
        .iter()
        .map(|found| found.task_id)
        .collect::<BTreeSet<_>>();
    views.retain(|view| !terminal(view.task.status) || recent.contains(&view.task.task_id));
    views.sort_by(|a, b| {
        a.task
            .spec
            .project
            .cmp(&b.task.spec.project)
            .then(a.task.task_id.cmp(&b.task.task_id))
    });
    let items_truncated = !complete || views.len() > limits.items;
    views.truncate(limits.items);
    events.sort_by_key(|found| (found.at, found.offset));
    let events_truncated = !complete || events.len() > limits.events;
    let excess = events.len().saturating_sub(limits.events);
    events.drain(..excess);
    Selection {
        views,
        items_truncated,
        events,
        events_truncated,
    }
}

/// 已结束＝成功、失败、取消、放弃；`approved` 与各 `needs_*` 仍算未结束。
fn terminal(status: TaskStatus) -> bool {
    matches!(
        status,
        TaskStatus::Succeeded | TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Abandoned
    )
}

/// 标题是否只由空白组成。空白取 Unicode White_Space 再加 U+FEFF：浏览器 `trim()` 会去掉
/// U+FEFF 而 Rust 不认它是空白；按并集判定，发出去的标题在两边都不是空白。
fn blank(title: &str) -> bool {
    title
        .chars()
        .all(|character| character.is_whitespace() || character == '\u{feff}')
}

/// 把一条 `BoardView` 投影成汇总项：只留元数据，标题缺失或全空白时回落任务 id。
fn item(view: BoardView, holder_lineage: Vec<LineageNode>) -> Result<Item, String> {
    let receipt_status = view
        .board
        .receipt
        .as_ref()
        .map(|receipt| {
            receipt
                .get("status")
                .cloned()
                .and_then(|status| serde_json::from_value::<ReceiptStatus>(status).ok())
                .ok_or("回执状态无效")
        })
        .transpose()?;
    let id = view.task.task_id;
    let active = view.task.active_attempt;
    Ok(Item {
        id,
        title: view
            .task
            .spec
            .title
            .filter(|title| !blank(title))
            .unwrap_or_else(|| id.to_string()),
        project: view.task.spec.project,
        status: view.task.status,
        parent: view.board.definition.parent,
        adapter: view.task.spec.adapter,
        holder: view.holder,
        holder_lineage,
        paths: view.board.definition.files_owned,
        attempt: view.attempt.map(|attempt| Attempt {
            phase: attempt.phase,
            active: active == Some(attempt.attempt_id),
            worker: attempt.worker_id,
            lease_until_ms: attempt.lease_until_ms,
        }),
        progress: view.progress.map(|progress| Progress {
            tools: progress.tool_calls,
            edits: progress.edits,
            last_progress_ms: progress.last_progress_ms,
        }),
        receipt_status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::AtomicUsize;
    use studiolo_audit_ledger::{
        AuditError, AuditEvent, AuditEventId, AuditSubject, IntegrityHash, SanitizedPayload,
    };
    use studiolo_control_db::{AttemptProgress, AttemptRecord, BoardDefinition, BoardRecord};
    use studiolo_core::Fence;
    use studiolo_task_engine::{RetryPolicy, TaskSpec, TaskView};

    const SMALL: Limits = Limits {
        items: 3,
        events: 4,
        page: 4,
        scan: 12,
    };

    fn at(second: i64) -> Timestamp {
        Timestamp::from_second(second).unwrap()
    }

    /// 按给定时刻重建记录的只读账本替身：真实账本只能盖「现在」的时间戳，
    /// 这里用它构造跨天的偏移序列。只实现汇总会用到的 after_offset 与 limit，
    /// 其余过滤条件出现即判测试失败；并记下整页读取（limit 不为 1）读到的最小偏移，
    /// 用来核对倒扫在越过窗口起点后确实停下。
    struct Fixed {
        records: Vec<AuditRecord>,
        lowest: AtomicUsize,
    }

    impl Fixed {
        fn new(entries: &[(i64, Option<TaskId>)]) -> Self {
            let records = entries
                .iter()
                .zip(1u64..)
                .map(|((second, task), offset)| {
                    let mut subject = AuditSubject::default();
                    if let Some(task) = task {
                        subject = subject.with_task(*task).with_attempt(AttemptId::new());
                    }
                    let event = AuditEvent::new(
                        AuditEventId::parse(format!("fixture:{offset}")).unwrap(),
                        AuditPhase::Outcome,
                        "fixture.progress",
                        SanitizedPayload::try_from(json!({})).unwrap(),
                    )
                    .with_subject(subject);
                    AuditRecord::from_parts(
                        offset,
                        at(*second),
                        IntegrityHash::genesis(),
                        IntegrityHash::genesis(),
                        event,
                    )
                })
                .collect();
            Self {
                records,
                lowest: AtomicUsize::new(usize::MAX),
            }
        }
    }

    impl AuditLedger for Fixed {
        fn append(&self, _event: AuditEvent) -> Result<AuditRecord, AuditError> {
            Err(AuditError::Unavailable)
        }

        fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError> {
            assert!(
                filter.event_id().is_none()
                    && filter.task_id().is_none()
                    && filter.attempt_id().is_none()
                    && filter.tool().is_none()
                    && filter.phase().is_none()
                    && filter.action().is_none(),
                "汇总只按偏移翻页"
            );
            let after = filter.offset_exclusive().unwrap_or(0);
            let found: Vec<_> = self
                .records
                .iter()
                .filter(|record| record.offset() > after)
                .take(filter.limit().unwrap_or(usize::MAX))
                .cloned()
                .collect();
            if let Some(first) = found.first()
                && filter.limit() != Some(1)
            {
                self.lowest
                    .fetch_min(first.offset() as usize, Ordering::Relaxed);
            }
            Ok(found)
        }
    }

    fn uniform(count: usize, second: i64) -> Fixed {
        Fixed::new(&vec![(second, None); count])
    }

    #[test]
    fn tail_floor_brackets_the_last_offset_from_any_hint() {
        for count in [0u64, 1, 3, 4, 5, 8, 9, 17, 40, 131] {
            let ledger = uniform(count as usize, 1_000);
            for hint in [0, count / 2, count, count + 1, count + 100, u64::MAX] {
                let floor = tail_floor(&ledger, hint, 4).unwrap();
                assert!(
                    floor <= count && count < floor + 4,
                    "记录数 {count} 提示 {hint} 起点 {floor}"
                );
            }
        }
    }

    #[test]
    fn gaps_in_the_ledger_fail_closed() {
        let mut ledger = uniform(6, 1_000);
        ledger.records.remove(2);
        assert!(present(&ledger, 3).is_err());
        assert!(page(&ledger, 0, 4).is_err());
    }

    fn window() -> (Timestamp, Timestamp) {
        (at(100_000), at(200_000))
    }

    #[test]
    fn scan_stops_at_the_first_page_older_than_the_window() {
        let (task, other) = (TaskId::new(), TaskId::new());
        let projects = HashMap::from([(task, "项目/A")]);
        let mut entries = vec![(10, Some(task)); 12];
        entries.extend([
            (150_000, Some(task)),
            (150_001, None),
            (150_002, Some(other)),
            (150_003, Some(task)),
            (250_000, Some(task)),
        ]);
        let ledger = Fixed::new(&entries);
        let hint = AtomicU64::new(0);
        let scanned = scan(&ledger, &hint, &projects, window(), SMALL).unwrap();
        assert!(scanned.complete);
        let offsets: Vec<_> = scanned.events.iter().map(|e| e.offset).collect();
        assert_eq!(offsets, vec![13, 16], "只收窗口内、主体是黑板任务的事件");
        assert!(scanned.events.iter().all(|e| e.project == "项目/A"));
        assert_eq!(hint.load(Ordering::Relaxed), 17, "尾部提示前移");
        assert_eq!(
            ledger.lowest.load(Ordering::Relaxed),
            9,
            "整页早于停扫线的那一页是最后读到的一页"
        );
    }

    #[test]
    fn clock_slack_tolerates_a_small_backward_step() {
        let task = TaskId::new();
        let projects = HashMap::from([(task, "项目/A")]);
        // 窗口内事件之后跟着一整页因时钟回拨而略早于窗口起点的记录。
        let mut entries = vec![(10, None); 3];
        entries.push((100_100, Some(task)));
        entries.extend([(99_950, None); 8]);
        let ledger = Fixed::new(&entries);
        let scanned = scan(&ledger, &AtomicU64::new(0), &projects, window(), SMALL).unwrap();
        assert!(scanned.complete);
        assert_eq!(scanned.events.len(), 1);
        assert_eq!(scanned.events[0].offset, 4);
    }

    #[test]
    fn scan_limit_marks_the_window_incomplete() {
        let task = TaskId::new();
        let projects = HashMap::from([(task, "项目/A")]);
        let ledger = Fixed::new(&[(150_000, Some(task)); 30]);
        let scanned = scan(&ledger, &AtomicU64::new(0), &projects, window(), SMALL).unwrap();
        assert!(!scanned.complete);
        assert!(scanned.events.len() as u64 >= SMALL.scan);
        let selection = select(vec![], scanned, SMALL);
        assert!(selection.items_truncated && selection.events_truncated);
    }

    #[test]
    fn scan_reaching_the_ledger_start_is_complete() {
        let task = TaskId::new();
        let projects = HashMap::from([(task, "项目/A")]);
        let ledger = Fixed::new(&[(150_000, Some(task)); 9]);
        let scanned = scan(&ledger, &AtomicU64::new(0), &projects, window(), SMALL).unwrap();
        assert!(scanned.complete);
        assert_eq!(scanned.events.len(), 9);
        let empty = uniform(0, 0);
        let nothing = scan(&empty, &AtomicU64::new(0), &projects, window(), SMALL).unwrap();
        assert!(nothing.complete && nothing.events.is_empty());
    }

    fn view(project: &str, status: TaskStatus) -> BoardView {
        let task_id = TaskId::new();
        BoardView {
            candidate_stages: vec![],
            candidate_receipt: None,
            task: TaskView {
                task_id,
                spec: TaskSpec {
                    role: "implementer".into(),
                    adapter: "claude".into(),
                    input_paths: vec![],
                    allow_web: vec![],
                    allow_db: vec![],
                    retry_policy: RetryPolicy {
                        max_attempts: 1,
                        backoff_seconds: 0,
                        retry_on: vec![],
                    },
                    timeout: 60,
                    project: project.into(),
                    parent: None,
                    title: Some("汇总".into()),
                    instruction: Some("不进汇总的指令正文".into()),
                },
                status,
                active_attempt: None,
                last_attempt: None,
                input_snapshot: None,
            },
            board: BoardRecord {
                task_id,
                definition: BoardDefinition {
                    acceptance_criteria: vec!["不进汇总的验收标准".into()],
                    files_owned: vec![VaultPath::parse(format!("{project}/src")).unwrap()],
                    max_children: 8,
                    policy_hash: "a".repeat(64),
                    parent: None,
                    artifact_target: None,
                },
                depth: 0,
                created_by_instance: None,
                handoff: None,
                receipt: None,
            },
            uses_old_policy: false,
            holder: None,
            progress: None,
            attempt: None,
        }
    }

    fn found(task_id: TaskId, offset: u64, second: i64) -> Found {
        Found {
            offset,
            at: at(second),
            project: "项目/A".into(),
            task_id,
            action: "fixture.progress".into(),
            phase: AuditPhase::Outcome,
            attempt_id: None,
        }
    }

    #[test]
    fn unfinished_items_stay_and_terminal_items_need_window_events() {
        let running = view("项目/B", TaskStatus::Running);
        let approved = view("项目/B", TaskStatus::Approved);
        let recent = view("项目/A", TaskStatus::Succeeded);
        let quiet = view("项目/A", TaskStatus::Failed);
        let draft = view("项目/A", TaskStatus::Draft);
        let events = vec![found(recent.task.task_id, 5, 150_000)];
        let expected_order = {
            let mut a = vec![recent.task.task_id, draft.task.task_id];
            a.sort();
            let mut b = vec![running.task.task_id, approved.task.task_id];
            b.sort();
            [a, b].concat()
        };
        let quiet_id = quiet.task.task_id;
        let limits = Limits { items: 10, ..SMALL };
        let selection = select(
            vec![running, approved, recent, quiet, draft],
            Scan {
                events,
                complete: true,
            },
            limits,
        );
        let ids: Vec<_> = selection.views.iter().map(|v| v.task.task_id).collect();
        assert_eq!(ids, expected_order, "(project, id) order");
        assert!(!ids.contains(&quiet_id));
        assert!(!selection.items_truncated && !selection.events_truncated);
    }

    #[test]
    fn item_cap_keeps_the_first_in_project_then_id_order() {
        let views: Vec<_> = (0..5).map(|_| view("项目/A", TaskStatus::Queued)).collect();
        let mut ids: Vec<_> = views.iter().map(|v| v.task.task_id).collect();
        ids.sort();
        let selection = select(
            views,
            Scan {
                events: vec![],
                complete: true,
            },
            SMALL,
        );
        let kept: Vec<_> = selection.views.iter().map(|v| v.task.task_id).collect();
        assert_eq!(kept, ids[..3]);
        assert!(selection.items_truncated && !selection.events_truncated);
    }

    #[test]
    fn events_ascend_by_time_then_offset_and_keep_the_newest() {
        let task = TaskId::new();
        let events = vec![
            found(task, 7, 150_004),
            found(task, 2, 150_001),
            found(task, 9, 150_002),
            found(task, 4, 150_002),
            found(task, 1, 150_000),
            found(task, 8, 150_005),
        ];
        let selection = select(
            vec![],
            Scan {
                events,
                complete: true,
            },
            SMALL,
        );
        let order: Vec<_> = selection.events.iter().map(|e| e.offset).collect();
        assert_eq!(order, vec![4, 9, 7, 8], "保留最新四条并升序");
        assert!(selection.events_truncated && !selection.items_truncated);
        let exact = select(
            vec![],
            Scan {
                events: vec![found(task, 3, 150_003), found(task, 1, 150_001)],
                complete: true,
            },
            SMALL,
        );
        assert!(!exact.events_truncated);
        assert_eq!(exact.events.len(), 2);
    }

    #[test]
    fn items_carry_metadata_only_with_explicit_attempt_state() {
        let mut current = view("项目/A", TaskStatus::Running);
        let attempt_id = AttemptId::new();
        current.task.active_attempt = Some(attempt_id);
        current.holder = Some("instance-1".into());
        current.attempt = Some(AttemptRecord {
            attempt_id,
            task_id: current.task.task_id,
            fence: Fence::new(3).unwrap(),
            phase: AttemptPhase::Running,
            worker_id: "instance-1".into(),
            snapshot_id: "b".repeat(64),
            lease_until_ms: 5_000,
            runner_ref_json: None,
            terminal: None,
        });
        current.progress = Some(AttemptProgress {
            tool_calls: 12,
            edits: 3,
            last_progress_ms: 4_000,
        });
        let chain = vec![LineageNode {
            id: "instance-1".into(),
            agent_type: "claude".into(),
        }];
        let value = serde_json::to_value(item(current, chain).unwrap()).unwrap();
        assert_eq!(
            value["attempt"],
            json!({"phase":"running","worker":"instance-1","active":true,"lease_until_ms":5_000})
        );
        assert_eq!(
            value["progress"],
            json!({"tools":12,"edits":3,"last_progress_ms":4_000})
        );
        assert_eq!(
            value["holder_lineage"],
            json!([{"id":"instance-1","agent_type":"claude"}])
        );
        assert_eq!(value["receipt_status"], json!(null));
        let text = value.to_string();
        assert!(!text.contains("不进汇总"), "不含指令与验收标准正文");

        let mut ended = view("项目/A", TaskStatus::Cancelled);
        let last = AttemptId::new();
        ended.task.last_attempt = Some(last);
        ended.task.spec.title = Some("  ".into());
        ended.attempt = Some(AttemptRecord {
            attempt_id: last,
            task_id: ended.task.task_id,
            fence: Fence::new(4).unwrap(),
            phase: AttemptPhase::Cancelled,
            worker_id: "instance-2".into(),
            snapshot_id: "b".repeat(64),
            lease_until_ms: 6_000,
            runner_ref_json: None,
            terminal: None,
        });
        ended.board.receipt = Some(json!({"status":"stopped","remaining":["正文不外露"]}));
        let id = ended.task.task_id.to_string();
        let value = serde_json::to_value(item(ended, vec![]).unwrap()).unwrap();
        assert_eq!(value["title"], json!(id), "空白标题回落任务 id");
        assert_eq!(value["attempt"]["active"], json!(false));
        assert_eq!(value["receipt_status"], json!("stopped"));
        assert!(!value.to_string().contains("正文不外露"));

        let mut corrupt = view("项目/A", TaskStatus::Failed);
        corrupt.board.receipt = Some(json!({"status":"done"}));
        assert!(item(corrupt, vec![]).is_err(), "未知回执状态失败关闭");
    }

    #[test]
    fn titles_blank_in_either_runtime_fall_back_to_the_id() {
        for title in ["", " ", "\u{3000}\t\n", "\u{85}", "\u{feff}", " \u{feff} "] {
            assert!(blank(title), "{title:?}");
        }
        for title in ["汇总", " 汇总 ", "\u{200b}", "a\u{feff}"] {
            assert!(!blank(title), "{title:?}");
        }
        let mut odd = view("项目/A", TaskStatus::Draft);
        odd.task.spec.title = Some("\u{feff}".into());
        let id = odd.task.task_id.to_string();
        assert_eq!(item(odd, vec![]).unwrap().title, id);
    }
}
