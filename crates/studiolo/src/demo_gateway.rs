//! 显式隔离的 `Demo` 数据源；`URL`、`SQL`、请求头与数据库路径一概不由 `Runner` 选择。
use crate::stub_gateway::demo_arguments;
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_core::ToolId;
use studiolo_gateway::{
    AttemptGuard, Clock, Gateway, GatewayError, JobCapability, MemoryGateway, ModelRequest,
    ModelResponse, PermissionLayers, ToolCall, ToolResult,
};

/// 由管理员提供的 `Demo` 数据源映射，绝不从 `Runner` 请求反序列化而来。
pub struct DemoGatewayConfig {
    /// 必须恰好是 `demo`；本适配器无法靠配置变成生产数据源。
    pub mode: String,
    /// 固定的数字地址回环端点，只提供 `/demo.json`；拒绝重定向。
    pub endpoint: SocketAddr,
    /// 已存在的绝对路径，文件名为 `demo.sqlite3`，以只读方式打开。
    pub database: PathBuf,
}
type Cache = BTreeMap<String, (ToolCall, Result<ToolResult, GatewayError>)>;
/// 位于既有 `Gateway` 授权接缝之后的合成本地数据源。
/// 内部的 `MemoryGateway` 审计准入；另有独立的 `effect` 事件在实际 `I/O` 前后各记一次。
/// 结果槽有上限且只在本进程内有效；不承诺重启后仍然保留。
pub struct DemoGateway {
    admission: MemoryGateway,
    audit: Arc<dyn AuditLedger>,
    endpoint: SocketAddr,
    database: Mutex<Connection>,
    calls: Mutex<Cache>,
}
impl DemoGateway {
    /// 只接受显式的 `Demo` 模式，且只打开固定的只读 `SQLite` 数据源。
    pub fn open(
        config: DemoGatewayConfig,
        guard: Arc<dyn AttemptGuard>,
        audit: Arc<dyn AuditLedger>,
        clock: Arc<dyn Clock>,
        permissions: PermissionLayers,
    ) -> Result<Self, GatewayError> {
        if config.mode != "demo"
            || !config.endpoint.ip().is_loopback()
            || config.endpoint.port() == 0
            || !config.database.is_absolute()
            || config
                .database
                .file_name()
                .is_none_or(|n| n != "demo.sqlite3")
        {
            return Err(GatewayError::ToolNotAllowed);
        }
        let database = Connection::open_with_flags(
            &config.database,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|_| GatewayError::ToolUnavailable)?;
        database
            .busy_timeout(Duration::from_secs(2))
            .map_err(|_| GatewayError::ToolUnavailable)?;
        database
            .execute_batch("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
            .map_err(|_| GatewayError::ToolUnavailable)?;
        let table: i64 = database
            .query_row(
                "SELECT count(*) FROM sqlite_schema WHERE type='table' AND name='demo_items'",
                [],
                |r| r.get(0),
            )
            .map_err(|_| GatewayError::ToolUnavailable)?;
        if table != 1 {
            return Err(GatewayError::ToolUnavailable);
        }
        let admission = MemoryGateway::new(guard, audit.clone(), clock, permissions);
        for name in ["web.fetch", "db.query"] {
            admission
                .set_tool_response(ToolId::from_static(name), json!({"demo_admission":true}))?;
        }
        Ok(Self {
            admission,
            audit,
            endpoint: config.endpoint,
            database: Mutex::new(database),
            calls: Mutex::new(BTreeMap::new()),
        })
    }
    fn event(
        &self,
        cap: &JobCapability,
        call: &ToolCall,
        phase: AuditPhase,
        allowed: bool,
    ) -> Result<(), GatewayError> {
        let id = AuditEventId::parse(format!(
            "demo.effect:{}:{}:{}:{}",
            cap.task_id(),
            cap.attempt_id(),
            cap.fence().get(),
            call.call_id().as_str()
        ))
        .map_err(|_| GatewayError::InvalidCallId)?;
        let payload = SanitizedPayload::try_from(json!({"mode":"demo","tool":call.tool().as_str(),
            "decision":if allowed {"allow"} else {"deny"}}))
        .map_err(|_| GatewayError::AuditUnavailable)?;
        self.audit
            .append(
                AuditEvent::new(id, phase, "gateway.demo.effect", payload).with_subject(
                    AuditSubject::default()
                        .with_task(cap.task_id())
                        .with_attempt(cap.attempt_id())
                        .with_tool(call.tool().clone()),
                ),
            )
            .map_err(|_| GatewayError::AuditUnavailable)?;
        Ok(())
    }
    fn query(&self) -> Result<Value, GatewayError> {
        let db = self
            .database
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        let mut statement = db.prepare("SELECT id, CASE WHEN length(CAST(label AS BLOB))<=1024 THEN label END, value FROM demo_items ORDER BY id LIMIT 33").map_err(|_| GatewayError::ToolUnavailable)?;
        let rows = statement
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                ))
            })
            .map_err(|_| GatewayError::ToolUnavailable)?;
        let mut output = Vec::new();
        for row in rows {
            let (id, label, value) = row.map_err(|_| GatewayError::ToolUnavailable)?;
            if output.len() >= 32 || label.len() > 1024 {
                return Err(GatewayError::ToolUnavailable);
            }
            output.push(json!({"id":id,"label":label,"value":value}));
        }
        Ok(json!({"mode":"demo","rows":output}))
    }
    fn fetch(&self) -> Result<Value, GatewayError> {
        let unavailable = |_| GatewayError::ToolUnavailable;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut stream = TcpStream::connect_timeout(&self.endpoint, Duration::from_secs(2))
            .map_err(unavailable)?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(unavailable)?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(unavailable)?;
        let request = format!(
            "GET /demo.json HTTP/1.1\r\nHost: {}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
            self.endpoint
        );
        stream.write_all(request.as_bytes()).map_err(unavailable)?;
        let mut headers = Vec::new();
        loop {
            stream
                .set_read_timeout(Some(
                    deadline
                        .checked_duration_since(Instant::now())
                        .filter(|d| !d.is_zero())
                        .ok_or(GatewayError::ToolUnavailable)?,
                ))
                .map_err(unavailable)?;
            let mut byte = [0];
            if stream.read(&mut byte).map_err(unavailable)? == 0 {
                return Err(GatewayError::ToolUnavailable);
            }
            headers.push(byte[0]);
            if headers.len() > 8192 {
                return Err(GatewayError::ToolUnavailable);
            }
            if headers.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let header = std::str::from_utf8(&headers).map_err(|_| GatewayError::ToolUnavailable)?;
        let mut lines = header.split("\r\n");
        if !lines.next().is_some_and(|line| {
            line.starts_with("HTTP/1.1 200 ") || line.starts_with("HTTP/1.0 200 ")
        }) {
            return Err(GatewayError::ToolUnavailable);
        }
        let mut length = None;
        for line in lines.filter(|s| !s.is_empty()) {
            let (name, value) = line.split_once(':').ok_or(GatewayError::ToolUnavailable)?;
            match name.to_ascii_lowercase().as_str() {
                "content-length" if length.is_none() => {
                    length = Some(
                        value
                            .trim()
                            .parse::<usize>()
                            .map_err(|_| GatewayError::ToolUnavailable)?,
                    )
                }
                "content-length" | "transfer-encoding" | "content-encoding" => {
                    return Err(GatewayError::ToolUnavailable);
                }
                _ => {}
            }
        }
        let size = length
            .filter(|n| *n <= 65536)
            .ok_or(GatewayError::ToolUnavailable)?;
        let mut bytes = vec![0; size];
        let mut read = 0;
        while read < size {
            stream
                .set_read_timeout(Some(
                    deadline
                        .checked_duration_since(Instant::now())
                        .filter(|d| !d.is_zero())
                        .ok_or(GatewayError::ToolUnavailable)?,
                ))
                .map_err(unavailable)?;
            let got = stream.read(&mut bytes[read..]).map_err(unavailable)?;
            if got == 0 {
                return Err(GatewayError::ToolUnavailable);
            }
            read += got;
        }
        serde_json::from_slice(&bytes).map_err(|_| GatewayError::ToolUnavailable)
    }
}
impl Gateway for DemoGateway {
    fn invoke(&self, cap: &JobCapability, call: ToolCall) -> Result<ToolResult, GatewayError> {
        let mut calls = self
            .calls
            .lock()
            .map_err(|_| GatewayError::StateUnavailable)?;
        // 复用的这道闸门检查全部四层权限、当前 `fence`、过期与取消。
        self.admission.invoke(cap, call.clone())?;
        let key = format!(
            "{}:{}:{}:{}",
            cap.task_id(),
            cap.attempt_id(),
            cap.fence().get(),
            call.call_id().as_str()
        );
        if let Some((prior, result)) = calls.get(&key) {
            return if prior == &call {
                result.clone()
            } else {
                Err(GatewayError::CallConflict)
            };
        }
        if calls.len() >= 128 {
            return Err(GatewayError::ToolUnavailable);
        }
        self.event(cap, &call, AuditPhase::Intent, true)?;
        let output = if call.arguments() != &demo_arguments(call.tool().as_str()) {
            Err(GatewayError::ToolNotAllowed)
        } else {
            match call.tool().as_str() {
                "web.fetch" => self.fetch(),
                "db.query" => self.query(),
                _ => Err(GatewayError::ToolNotAllowed),
            }
        };
        // `I/O` 期间发生的取消既不能放出这次响应，也不能产生成功的候选。
        let mut result = output.and_then(|output| {
            self.admission.invoke(cap, call.clone())?;
            Ok(ToolResult::new(
                call.call_id().clone(),
                call.tool().clone(),
                output,
            ))
        });
        if self
            .event(cap, &call, AuditPhase::Outcome, result.is_ok())
            .is_err()
        {
            result = Err(GatewayError::AuditUnavailable);
        }
        calls.insert(key, (call, result.clone()));
        result
    }
    fn relay(
        &self,
        _cap: &JobCapability,
        _request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError> {
        Err(GatewayError::ModelUnavailable)
    }
}
