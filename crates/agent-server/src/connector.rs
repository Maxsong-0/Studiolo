//! 连接器入口 `/connect/v1/*`（ADR-046）：给只有窄权限的机器程序，无会话、不握手、不发路牌、不经供给副本。
//!
//! 每个请求验类型令牌、复核策略，再经私有 Unix 入口交给权威服务（那里按身份库、策略库再裁一遍）。看黑板在
//! 交付前按项目记 `content_read`，304 没给任何内容所以不记；推状态这边不记，人侧只登记新会话。
mod limits;
mod reply;

use crate::{
    auth::{self, AgentType, CredentialDigest, Unrecognized},
    board_client::ConnectorError,
    denials::Denials,
    http_text::strict_query,
    shutdown::SERVER_STOPPING,
    state::{AppState, now},
};
use axum::{
    Json, Router,
    body::{Body, Bytes, to_bytes},
    extract::State,
    http::{HeaderMap, HeaderValue, Method, Request, StatusCode, header, request::Parts},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use limits::Limits;
use reply::{Refusal, *};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use studiolo_policy::CapabilitySet;
use studiolo_supply::audit::{self, Actor};
use tokio::sync::Semaphore;

/// 一份快照的正文上限。
const MAX_BODY: usize = 64 * 1024;
/// 正文多久还没收全就不等了：先收完再占名额，断在半截的连接拖不住别的请求。
const BODY_WAIT: Duration = Duration::from_secs(10);
/// 同时在途（含排队）的连接器请求数，再多就 429。
const MAX_IN_FLIGHT: usize = 4;
/// 其中同时交给权威服务的；人侧也只给连接器一半黑板名额（`board/mod.rs` 的 `MAX_CONNECTORS`），超了会被当成忙。
const MAX_AT_AUTHORITY: usize = 2;
/// 没认出类型的拒绝登记在这个空名字下。
const ANONYMOUS: &str = "";

struct Connect {
    app: Arc<AppState>,
    limits: Mutex<Limits>,
    denials: Mutex<Denials>,
    requests: Semaphore,
    authority: Semaphore,
    /// 本进程起来之后每个连接器类型登记的那一个实例。
    instances: Mutex<BTreeMap<String, Registered>>,
}

/// 已写进身份库的实例；登记审计没成功之前不能用，下次先补这条，不再另登记一个。
#[derive(Clone)]
struct Registered {
    id: String,
    audited: bool,
}

pub(crate) fn router(app: Arc<AppState>) -> Router {
    let state = Arc::new(Connect {
        app,
        limits: Mutex::new(Limits::default()),
        denials: Mutex::new(Denials::default()),
        requests: Semaphore::new(MAX_IN_FLIGHT),
        authority: Semaphore::new(MAX_AT_AUTHORITY),
        instances: Mutex::new(BTreeMap::new()),
    });
    let routes = Router::new()
        .fallback(dispatch)
        .with_state(state.clone())
        .layer(middleware::from_fn_with_state(state, gate))
        .layer(middleware::from_fn(mark));
    Router::new().nest("/connect", routes)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Entry {
    Board,
    Task,
    Sessions,
}

impl Entry {
    fn find(path: &str) -> Option<Self> {
        match path {
            "/v1/board" => Some(Self::Board),
            "/v1/board/task" => Some(Self::Task),
            "/v1/sessions" => Some(Self::Sessions),
            _ => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Board => "connect.board",
            Self::Task => "connect.board.task",
            Self::Sessions => "connect.sessions",
        }
    }
    fn method(self) -> Method {
        match self {
            Self::Board | Self::Task => Method::GET,
            Self::Sessions => Method::PUT,
        }
    }
    /// 这个入口看能力集里哪张名单。
    fn roster(self, set: &CapabilitySet) -> &[String] {
        match self {
            Self::Board | Self::Task => &set.read.board_projects,
            Self::Sessions => &set.write.presence_projects,
        }
    }
}

/// 已认证的调用方。
struct Caller {
    agent_type: String,
    digest: String,
}

/// 复核通过后的名单与策略哈希。
#[derive(Clone, Debug, PartialEq, Eq)]
struct Granted {
    roster: Vec<String>,
    effective_hash: String,
    /// 这枚令牌是哪一次签发的；人侧据此把轮换当撤权。
    issued_at: String,
}

/// 认证门：和别的入口同一套认法；401、503 的登记一样限频，只是算在这个入口自己名下。
async fn gate(
    State(state): State<Arc<Connect>>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    match auth::recognize(&state.app, request.headers()) {
        Ok((agent_type, digest)) => {
            request.extensions_mut().insert(agent_type);
            request.extensions_mut().insert(digest);
            next.run(request).await
        }
        Err(refused) => {
            state.deny_anonymous(refused);
            refused.into_response()
        }
    }
}

async fn dispatch(State(state): State<Arc<Connect>>, request: Request<Body>) -> Response {
    let caller = match (
        request.extensions().get::<AgentType>(),
        request.extensions().get::<CredentialDigest>(),
    ) {
        (Some(AgentType(agent_type)), Some(CredentialDigest(digest))) => Caller {
            agent_type: agent_type.clone(),
            digest: digest.clone(),
        },
        _ => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let Some(entry) = Entry::find(request.uri().path()) else {
        return NO_ROUTE.into_response();
    };
    if request.method() != entry.method() {
        return METHOD.into_response();
    }
    // 关停开始后不接新请求；已在处理的照常做完。
    if state.app.lifecycle.is_stopping() {
        state.deny(&caller, entry, SERVER_STOPPING);
        return STOPPING.into_response();
    }
    let admitted = match state.limits.lock() {
        Ok(mut limits) => limits.admit(&caller.agent_type, entry.name(), Instant::now()),
        Err(_) => return UNAVAILABLE.into_response(),
    };
    if let Err(secs) = admitted {
        return TOO_MANY.after(secs).into_response();
    }
    let (parts, body) = request.into_parts();
    let body = match entry {
        Entry::Board | Entry::Task => Bytes::new(),
        Entry::Sessions => match read(body).await {
            Ok(body) => body,
            Err(refusal) => return refusal.into_response(),
        },
    };
    let Ok(_permit) = state.requests.try_acquire() else {
        return TOO_MANY.after(5).into_response();
    };
    let Ok(_slot) = state.authority.acquire().await else {
        return UNAVAILABLE.into_response();
    };
    // 整个请求持着操作准入：封存冻结时既不写身份库，也不在异步线程里同步干等账本。
    let Some(_operation) = state.app.begin_operation().await else {
        return UNAVAILABLE.into_response();
    };
    let reply = match entry {
        Entry::Board => state.board(&caller, &parts).await,
        Entry::Task => state.task(&caller, &parts).await,
        Entry::Sessions => state.sessions(&caller, &parts, &body).await,
    };
    reply.unwrap_or_else(IntoResponse::into_response)
}

/// 收完正文；超大、超时各有说法。
async fn read(body: Body) -> Result<Bytes, Refusal> {
    match tokio::time::timeout(BODY_WAIT, to_bytes(body, MAX_BODY)).await {
        Ok(Ok(body)) => Ok(body),
        Ok(Err(_)) => Err(TOO_LARGE),
        Err(_) => Err(TIMEOUT),
    }
}

impl Connect {
    async fn board(&self, caller: &Caller, request: &Parts) -> Result<Response, Refusal> {
        strict_query(request.uri.query(), &[], &[]).map_err(|()| INVALID)?;
        let (instance, granted) = self.enter(caller, Entry::Board)?;
        let reply = self
            .call(caller, &instance, &granted, "board", json!({}), INVALID)
            .await?;
        // 返回前再复核：排队和执行期间撤了权，就一项都不给。
        self.recheck(caller, Entry::Board, &instance, &granted)?;
        let projects = reply.get("projects").ok_or(UNAVAILABLE)?;
        let named = projects
            .as_array()
            .ok_or(UNAVAILABLE)?
            .iter()
            .map(|project| project.get("project").and_then(Value::as_str))
            .collect::<Option<Vec<_>>>()
            .ok_or(UNAVAILABLE)?;
        // 权威服务不该多给；真多给了也不往外送。
        if named
            .iter()
            .any(|name| !granted.roster.iter().any(|allowed| allowed == name))
        {
            return Err(UNAVAILABLE);
        }
        let etag = etag(projects)?;
        if unchanged(&request.headers, &etag) {
            let mut response = StatusCode::NOT_MODIFIED.into_response();
            response.headers_mut().insert(header::ETAG, etag);
            return Ok(response);
        }
        for name in named {
            self.record(caller, &instance, &granted, name, Entry::Board)?;
        }
        let mut response = Json(reply).into_response();
        response.headers_mut().insert(header::ETAG, etag);
        Ok(response)
    }

    async fn task(&self, caller: &Caller, request: &Parts) -> Result<Response, Refusal> {
        let names = ["project", "task_id"];
        let query = strict_query(request.uri.query(), &names, &names).map_err(|()| INVALID)?;
        let (instance, granted) = self.enter(caller, Entry::Task)?;
        let project = &query["project"];
        if !granted.roster.contains(project) {
            self.deny(caller, Entry::Task, "outside_roster");
            return Err(NOT_FOUND);
        }
        let reply = self
            .call(
                caller,
                &instance,
                &granted,
                "task",
                json!({"task_id": query["task_id"]}),
                INVALID,
            )
            .await?;
        self.recheck(caller, Entry::Task, &instance, &granted)?;
        if reply.pointer("/task/spec/project").and_then(Value::as_str) != Some(project) {
            return Err(NOT_FOUND);
        }
        self.record(caller, &instance, &granted, project, Entry::Task)?;
        Ok(Json(reply).into_response())
    }

    async fn sessions(
        &self,
        caller: &Caller,
        request: &Parts,
        body: &Bytes,
    ) -> Result<Response, Refusal> {
        strict_query(request.uri.query(), &[], &[]).map_err(|()| INVALID)?;
        let (instance, granted) = self.enter(caller, Entry::Sessions)?;
        let json = request
            .headers
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value.as_bytes().starts_with(b"application/json"));
        if !json {
            return Err(INVALID);
        }
        // 这里只核是不是一个 JSON 对象；字段由权威服务一处裁决。
        let snapshot = serde_json::from_slice::<Value>(body)
            .ok()
            .filter(Value::is_object)
            .ok_or(INVALID)?;
        let reply = self
            .call(
                caller,
                &instance,
                &granted,
                "sessions",
                snapshot,
                INVALID_SESSIONS,
            )
            .await?;
        // 已经落下的改不了了；撤权之后读的那一侧当场就没，这里只是不再回「收下了」。
        self.recheck(caller, Entry::Sessions, &instance, &granted)?;
        Ok(Json(reply).into_response())
    }

    /// 复核策略并取得本进程给这个类型登记的实例；没有权限的类型不登记实例。
    fn enter(&self, caller: &Caller, entry: Entry) -> Result<(String, Granted), Refusal> {
        self.granted(caller, entry, None)?;
        let instance = self.instance(caller)?;
        let granted = self.granted(caller, entry, Some(&instance))?;
        let _ = self.app.store.touch_instance(&instance, &now());
        Ok((instance, granted))
    }

    fn granted(
        &self,
        caller: &Caller,
        entry: Entry,
        instance: Option<&str>,
    ) -> Result<Granted, Refusal> {
        // 没接策略平面就没有任何能力集，谁都没有连接器权限。
        let Some(policy) = &self.app.policy else {
            return Err(self.refuse(caller, entry, "policy_not_configured"));
        };
        let authorization = policy
            .authorization(&caller.agent_type, instance)
            .map_err(|_| UNAVAILABLE)?;
        if let Some(denial) = authorization.denial {
            return Err(self.refuse(caller, entry, denial.as_str()));
        }
        let set = authorization.grant.ok_or(UNAVAILABLE)?.set;
        let roster = entry.roster(&set);
        if roster.is_empty() {
            return Err(self.refuse(caller, entry, "connector_not_granted"));
        }
        Ok(Granted {
            roster: roster.to_vec(),
            effective_hash: authorization.effective_hash,
            issued_at: self.credential(caller, entry)?,
        })
    }

    /// 令牌这一刻仍是现行的，连同它的签发时刻。
    fn credential(&self, caller: &Caller, entry: Entry) -> Result<String, Refusal> {
        self.app
            .store
            .credential_issued_at(&caller.agent_type, &caller.digest)
            .map_err(|_| UNAVAILABLE)?
            .ok_or_else(|| self.refuse(caller, entry, "credential_revoked"))
    }

    fn recheck(
        &self,
        caller: &Caller,
        entry: Entry,
        instance: &str,
        before: &Granted,
    ) -> Result<(), Refusal> {
        if self.granted(caller, entry, Some(instance))?.roster == before.roster {
            Ok(())
        } else {
            Err(UNAVAILABLE)
        }
    }

    fn instance(&self, caller: &Caller) -> Result<String, Refusal> {
        let mut instances = self.instances.lock().map_err(|_| UNAVAILABLE)?;
        let known = instances.get(&caller.agent_type).cloned();
        let instance = match known {
            Some(known) if known.audited => return Ok(known.id),
            Some(known) => known.id,
            None => {
                let instance = self
                    .app
                    .store
                    .register_instance(
                        &caller.agent_type,
                        &format!("connector:{}", ulid::Ulid::generate()),
                        &now(),
                    )
                    .map_err(|_| UNAVAILABLE)?;
                instance.id
            }
        };
        let mut remember = |audited| {
            let registered = Registered {
                id: instance.clone(),
                audited,
            };
            instances.insert(caller.agent_type.clone(), registered);
        };
        remember(false);
        let event = audit::instance_registered(Actor::instance(&caller.agent_type, &instance))
            .map_err(|_| UNAVAILABLE)?;
        self.app.audit(event).map_err(|_| UNAVAILABLE)?;
        remember(true);
        Ok(instance)
    }

    /// `invalid` 是权威服务说「字段不合」时对外的答复：读是 400，推是 422。
    async fn call(
        &self,
        caller: &Caller,
        instance: &str,
        granted: &Granted,
        operation: &str,
        request: Value,
        invalid: Refusal,
    ) -> Result<Value, Refusal> {
        let client = self.app.board.as_ref().ok_or(UNAVAILABLE)?;
        client
            .connect(&json!({
                "agent_type": caller.agent_type, "instance_id": instance,
                "issued_at": granted.issued_at, "operation": operation, "request": request,
            }))
            .await
            .map_err(|error| match error {
                ConnectorError::Denied => NOT_ALLOWED,
                ConnectorError::NotFound => NOT_FOUND,
                ConnectorError::Stale => STALE,
                ConnectorError::Invalid => invalid,
                ConnectorError::Unavailable => UNAVAILABLE,
            })
    }

    /// 内容交付前的审计：登记不了就一项不给。
    fn record(
        &self,
        caller: &Caller,
        instance: &str,
        granted: &Granted,
        project: &str,
        entry: Entry,
    ) -> Result<(), Refusal> {
        let event = audit::content_read(
            Actor::instance(&caller.agent_type, instance),
            project,
            &granted.effective_hash,
            entry.name(),
        )
        .map_err(|_| UNAVAILABLE)?;
        self.app.audit(event).map_err(|_| UNAVAILABLE)?;
        Ok(())
    }

    fn refuse(&self, caller: &Caller, entry: Entry, reason: &'static str) -> Refusal {
        self.deny(caller, entry, reason);
        NOT_ALLOWED
    }

    fn deny(&self, caller: &Caller, entry: Entry, reason: &'static str) {
        let actor = Actor {
            agent_type: Some(&caller.agent_type),
            instance_id: None,
        };
        self.note_denial(&caller.agent_type, actor, entry.name(), reason);
    }

    fn deny_anonymous(&self, refused: Unrecognized) {
        let actor = Actor::anonymous();
        self.note_denial(ANONYMOUS, actor, "connect.request", refused.reason());
    }

    /// 拒绝登记：同一（类型，原因）每分钟一条；登记不了不改变拒绝的结论。
    fn note_denial(&self, slot: &str, actor: Actor<'_>, action: &str, reason: &'static str) {
        let due = self
            .denials
            .lock()
            .is_ok_and(|mut denials| denials.due(slot, reason, Instant::now()));
        if !due {
            return;
        }
        if let Ok(event) = audit::action_denied(actor, action, reason) {
            let _ = self.app.audit(event);
        }
    }
}

/// 摘要只含各项目的内容，不含生成时刻：没变就是同一个。
fn etag(projects: &Value) -> Result<HeaderValue, Refusal> {
    let bytes = serde_json::to_vec(projects).map_err(|_| UNAVAILABLE)?;
    let digest = Sha256::digest(&bytes)
        .iter()
        .take(16)
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    HeaderValue::from_str(&format!("\"{digest}\"")).map_err(|_| UNAVAILABLE)
}

/// `If-None-Match` 里有没有这个标签；边缘压缩会把它改成弱标签（`W/`），照样算。
fn unchanged(headers: &HeaderMap, etag: &HeaderValue) -> bool {
    headers
        .get_all(header::IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|tag| {
            let tag = tag.trim();
            tag.strip_prefix("W/").unwrap_or(tag).as_bytes() == etag.as_bytes()
        })
}

// 共用的 MCP 夹具里有本模块用不到的方法。
#[cfg(test)]
#[allow(dead_code)]
#[path = "../tests/connector_private/stopping.rs"]
mod tests;
