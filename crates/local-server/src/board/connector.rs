//! 连接器（ADR-046）的人侧一半：只读黑板、收「在做什么」状态、把状态给人和 AI 看。
//!
//! 私有入口 `/internal/connector` 挂在和 `/internal/board` 同一个 Unix 路由、同一道 peer 门里；这里再按身份库、
//! 策略库复核一遍，agent 只转发。读黑板不在人侧账本另记（agent 账本按项目记 `content_read`）；推状态只登记
//! 「新会话第一次出现」和限频的拒绝，更新与过期不进账本。
use super::{
    BoardPlane,
    auth::{Grant, Refusal},
    execute::{Failure as BoardFailure, parse, value},
    http::{busy, unavailable, unconfigured},
    overview::{Item, Lineages, terminal},
    presence::{EXPIRES, Fresh, Presence, Refused, Snapshot},
    requests::{self, Task},
};
use crate::editing::Problem;
use axum::{
    Extension, Json,
    extract::{Query, rejection::JsonRejection, rejection::QueryRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{AuditError, AuditEvent, AuditEventId, AuditPhase, SanitizedPayload};
use studiolo_policy::CapabilitySet;

/// 一个项目的列表最多给这么多条没结束的。
const MAX_ITEMS: usize = 200;
/// 整份列表的正文上限；agent 侧读到 1 MiB 就放弃，这里先收住并标 `truncated`。
const MAX_LIST_BYTES: usize = 700 * 1024;

/// 状态口连同它的钟：从这个进程开黑板算起。
pub(super) struct PresenceDesk {
    started: Instant,
    /// 内存从几点起攒的；人侧重启后状态空着，读的人要分得清「没有」和「还没推来」。
    since: String,
    state: Mutex<Presence>,
}
impl PresenceDesk {
    pub(super) fn open(now: jiff::Timestamp) -> Self {
        Self {
            started: Instant::now(),
            since: now.to_string(),
            state: Mutex::new(Presence::default()),
        }
    }
    fn lock(&self) -> Result<MutexGuard<'_, Presence>, Refusal> {
        self.state
            .lock()
            .map_err(|_| Refusal::Unavailable("状态口不可用"))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Call {
    agent_type: String,
    instance_id: String,
    operation: Operation,
    request: Value,
}
#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Operation {
    /// 名单内各项目没结束的工作项，只给元数据。
    Board,
    /// 名单内一条工作项的全文。
    Task,
    /// 一份整份会话状态快照。
    Sessions,
}

/// 私有入口的失败；代码由 agent 换成对外状态码。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Failure {
    /// 身份、策略确定不放行。
    Denied,
    /// 名单里没有这条，和不存在同一句话。
    NotFound,
    /// `seq` 不比上一份大。
    Stale,
    /// 字段不合。
    Invalid,
    /// 新会话登记得太多。
    Throttled,
    /// 这一刻读不出来、写不了登记。
    Unavailable,
}
impl From<Refusal> for Failure {
    fn from(refusal: Refusal) -> Self {
        match refusal {
            Refusal::Denied(_) => Self::Denied,
            Refusal::Unavailable(_) => Self::Unavailable,
        }
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let (status, code, error) = match self {
            Self::Denied => (
                StatusCode::FORBIDDEN,
                "connector_denied",
                "没有这个连接器权限。",
            ),
            Self::NotFound => (
                StatusCode::NOT_FOUND,
                "connector_not_found",
                "没有这条工作项。",
            ),
            Self::Stale => (
                StatusCode::CONFLICT,
                "connector_stale",
                "seq 不比上一份大，这一份没有收。",
            ),
            Self::Invalid => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "connector_invalid",
                "字段不合，整份没有收，上一份不变。",
            ),
            Self::Throttled => (
                StatusCode::TOO_MANY_REQUESTS,
                "connector_throttled",
                "新会话太多，稍后再推。",
            ),
            Self::Unavailable => return unavailable(),
        };
        Problem::new(status, code, error).into_response()
    }
}

#[derive(Serialize)]
struct ProjectItems {
    project: String,
    items: Vec<Item>,
    finished: usize,
    truncated: bool,
}

impl BoardPlane {
    fn connect(&self, call: Call) -> Result<Value, Failure> {
        self.storage.check().map_err(|_| Failure::Unavailable)?;
        match call.operation {
            Operation::Board => self.connector_board(&call),
            Operation::Task => self.connector_task(&call),
            Operation::Sessions => self.connector_sessions(call),
        }
    }

    /// 连接器复核：类型、实例、能力集都有效，且对应的名单非空。确定拒绝的限频登记。
    fn connector_grant(
        &self,
        call: &Call,
        roster: fn(&CapabilitySet) -> &[String],
    ) -> Result<Grant, Failure> {
        self.rostered(call, roster).map_err(|refusal| {
            if let (Refusal::Denied(reason), Ok(mut presence)) = (refusal, self.presence.lock()) {
                self.note_denial(call, reason, &mut presence);
            }
            refusal.into()
        })
    }

    fn rostered(
        &self,
        call: &Call,
        roster: fn(&CapabilitySet) -> &[String],
    ) -> Result<Grant, Refusal> {
        self.granted(&call.agent_type, &call.instance_id, |set| {
            if roster(set).is_empty() {
                Err(Refusal::Denied("能力集没有这个连接器权限"))
            } else {
                Ok(())
            }
        })
    }

    fn connector_board(&self, call: &Call) -> Result<Value, Failure> {
        if call.request != json!({}) {
            return Err(Failure::Invalid);
        }
        let grant = self.connector_grant(call, board_roster)?;
        let generated_at = jiff::Timestamp::now().to_string();
        let mut roster = grant.set.read.board_projects.clone();
        roster.sort();
        // 只在读工作项时持有引擎锁；实例链查询不阻塞黑板写入。
        let views = {
            let engine = self.engine.lock().map_err(|_| Failure::Unavailable)?;
            roster
                .into_iter()
                .map(|project| {
                    let views = engine
                        .board_list(&project, &grant.hash)
                        .map_err(|_| Failure::Unavailable)?;
                    Ok((project, views))
                })
                .collect::<Result<Vec<_>, Failure>>()?
        };
        let mut lineages = Lineages::default();
        let mut budget = MAX_LIST_BYTES;
        let mut projects = Vec::with_capacity(views.len());
        for (project, mut views) in views {
            let total = views.len();
            views.retain(|view| !terminal(view.task.status));
            views.sort_by_key(|view| view.task.task_id);
            let finished = total - views.len();
            let mut truncated = views.len() > MAX_ITEMS;
            let mut items = Vec::new();
            for view in views.into_iter().take(MAX_ITEMS) {
                let item = lineages
                    .project(self, view)
                    .map_err(|_| Failure::Unavailable)?;
                let size = serde_json::to_vec(&item)
                    .map_err(|_| Failure::Unavailable)?
                    .len();
                let Some(left) = budget.checked_sub(size) else {
                    truncated = true;
                    break;
                };
                budget = left;
                items.push(item);
            }
            projects.push(ProjectItems {
                project,
                items,
                finished,
                truncated,
            });
        }
        self.confirm(call, board_roster, &grant)?;
        Ok(json!({
            "generated_at": generated_at,
            "projects": serde_json::to_value(projects).map_err(|_| Failure::Unavailable)?,
        }))
    }

    fn connector_task(&self, call: &Call) -> Result<Value, Failure> {
        let grant = self.connector_grant(call, board_roster)?;
        let Task { task_id } = parse(&call.request).map_err(|_| Failure::Invalid)?;
        let view = self
            .engine
            .lock()
            .map_err(|_| Failure::Unavailable)?
            .board_detail(task_id, &grant.hash)
            .map_err(|_| Failure::Unavailable)?
            .filter(|view| {
                grant
                    .set
                    .read
                    .board_projects
                    .contains(&view.task.spec.project)
            })
            .ok_or(Failure::NotFound)?;
        self.confirm(call, board_roster, &grant)?;
        value(view).map_err(|_| Failure::Unavailable)
    }

    /// 返回前再复核：等锁期间撤了权或名单变了，就一项都不给，让对方重来。
    fn confirm(
        &self,
        call: &Call,
        roster: fn(&CapabilitySet) -> &[String],
        before: &Grant,
    ) -> Result<(), Failure> {
        let now = self.connector_grant(call, roster)?;
        if roster(&now.set) == roster(&before.set) {
            Ok(())
        } else {
            Err(Failure::Unavailable)
        }
    }

    fn connector_sessions(&self, call: Call) -> Result<Value, Failure> {
        // 授权、核对、登记、落下在同一次持锁里做完：不会有「核过之后又被撤权」的那一份落下来。
        let mut presence = self.presence.lock()?;
        let grant = self.rostered(&call, presence_roster).map_err(|refusal| {
            if let Refusal::Denied(reason) = refusal {
                self.note_denial(&call, reason, &mut presence);
            }
            Failure::from(refusal)
        })?;
        let Some(snapshot) = Snapshot::parse(call.request.clone()) else {
            return Err(Failure::Invalid);
        };
        let now = self.presence.started.elapsed();
        let planned = presence
            .plan(
                &call.agent_type,
                snapshot,
                &grant.set.write.presence_projects,
                now,
            )
            .map_err(|refused| match refused {
                Refused::Stale => Failure::Stale,
                Refused::Throttled => Failure::Throttled,
                Refused::Full => Failure::Unavailable,
            })?;
        if !planned.fresh.is_empty() {
            // 封存正冻结着审计时不等：含新会话的整份不收，只含老编号的照收。
            let _operation = self
                .audit
                .try_enter_operation()
                .map_err(|_| Failure::Unavailable)?
                .ok_or(Failure::Unavailable)?;
            for fresh in &planned.fresh {
                self.record_fresh(&call, fresh)
                    .map_err(|_| Failure::Unavailable)?;
                presence.note_record(&call.agent_type, now);
            }
        }
        if !planned.rejected.is_empty() {
            self.note_denial(&call, "项目不在名单", &mut presence);
        }
        let reply = json!({
            "accepted": planned.accepted(),
            "rejected": planned.rejected.iter()
                .map(|index| json!({"index": index, "reason": "project_not_allowed"}))
                .collect::<Vec<_>>(),
            "expires_in_secs": EXPIRES.as_secs(),
        });
        presence.commit(&call.agent_type, &call.instance_id, planned, now);
        Ok(reply)
    }

    /// 拒绝登记：同一（类型，原因）每分钟最多一条。什么都没放行，登记不了就跳过，不改变拒绝的结论。
    fn note_denial(&self, call: &Call, reason: &'static str, presence: &mut Presence) {
        let now = self.presence.started.elapsed();
        if !presence.denial_due(&call.agent_type, reason, now) {
            return;
        }
        if let Ok(Some(_operation)) = self.audit.try_enter_operation() {
            let _ = self.record(json!({
                "agent_type": call.agent_type, "agent_instance": call.instance_id,
                "reason": reason,
            }));
        }
    }

    fn record_fresh(&self, call: &Call, fresh: &Fresh) -> Result<(), AuditError> {
        self.record(json!({
            "agent_type": call.agent_type, "agent_instance": call.instance_id,
            "project": fresh.project, "vendor": fresh.vendor, "session_count": fresh.count,
        }))
    }

    fn record(&self, payload: Value) -> Result<(), AuditError> {
        let payload = SanitizedPayload::try_from(payload)?;
        let id = AuditEventId::parse(format!("board-presence:{}", ulid::Ulid::generate()))?;
        self.audit
            .append(AuditEvent::new(
                id,
                AuditPhase::Outcome,
                "board.presence",
                payload,
            ))
            .map(|_| ())
    }

    /// 现行状态：先按此刻的授权收窄、清过期，再给。授权读不出来就一条不给。
    fn presence_view(&self, project: Option<&str>) -> Result<Value, Refusal> {
        let mut presence = self.presence.lock()?;
        let now = self.presence.started.elapsed();
        self.refresh_presence(&mut presence, now)?;
        let sessions = serde_json::to_value(presence.view(now, project))
            .map_err(|_| Refusal::Unavailable("状态编码失败"))?;
        Ok(json!({
            "since": self.presence.since,
            "expires_in_secs": EXPIRES.as_secs(),
            "sessions": sessions,
        }))
    }

    /// 撤了权的来源整份删掉，收回的项目当场删。
    fn refresh_presence(&self, presence: &mut Presence, now: Duration) -> Result<(), Refusal> {
        presence.sweep(now);
        for (source, instance) in presence.sources() {
            match self.granted(&source, &instance, |_| Ok(())) {
                Ok(grant) => {
                    let roster = presence_roster(&grant.set);
                    presence.restrict(&source, (!roster.is_empty()).then_some(roster));
                }
                Err(Refusal::Denied(_)) => presence.restrict(&source, None),
                Err(refusal) => return Err(refusal),
            }
        }
        Ok(())
    }

    /// 巡检顺手清：授权读不出来时留到下一轮，读的人那边照样失败关闭。
    pub(super) fn sweep_presence(&self) {
        if let Ok(mut presence) = self.presence.lock() {
            let now = self.presence.started.elapsed();
            let _ = self.refresh_presence(&mut presence, now);
        }
    }

    /// `board` 工具的 `presence` 操作：只给请求的那个项目。
    pub(super) fn presence_for(&self, request: &requests::Request) -> Result<Value, BoardFailure> {
        if request.request != json!({}) {
            return Err("presence不接受额外参数".into());
        }
        Ok(self.presence_view(Some(request.project.as_str()))?)
    }
}

fn board_roster(set: &CapabilitySet) -> &[String] {
    &set.read.board_projects
}
fn presence_roster(set: &CapabilitySet) -> &[String] {
    &set.write.presence_projects
}

/// 私有连接器入口；正文拒绝沿用原提取器的状态码，代码固定，agent 据此换成对外答复。
pub(super) async fn command(
    Extension(plane): Extension<Arc<BoardPlane>>,
    call: Result<Json<Call>, JsonRejection>,
) -> Response {
    let Ok(Json(call)) = call else {
        return Failure::Invalid.into_response();
    };
    let Ok(permit) = plane.permits.clone().try_acquire_owned() else {
        return busy();
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        plane.connect(call)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(failure)) => failure.into_response(),
        Err(_) => unavailable(),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NoQuery {}

/// 人侧只读：`GET /api/v1/board/presence`，登录会话门禁内，不记审计。
pub(super) async fn presence(
    plane: Option<Extension<Arc<BoardPlane>>>,
    query: Result<Query<NoQuery>, QueryRejection>,
) -> Response {
    if query.is_err() {
        return crate::invalid_query("invalid_board_presence_query");
    }
    let Some(Extension(plane)) = plane else {
        return unconfigured();
    };
    let Ok(permit) = plane.permits.clone().try_acquire_owned() else {
        return busy();
    };
    let result = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        plane.presence_view(None)
    })
    .await;
    match result {
        Ok(Ok(value)) => Json(value).into_response(),
        _ => unavailable(),
    }
}
