//! 持久化的真实调用协议，与 MemoryGateway 的响应缓存相互独立。
use crate::{
    AttemptGuard, CallId, Clock, PermissionLayers, relay_request,
    relay_store::{Admission, Store, hash},
    relay_transport::RelayTransport,
    relay_types::*,
};
use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use studiolo_audit_ledger::{
    AuditEvent, AuditEventId, AuditLedger, AuditPhase, AuditSubject, SanitizedPayload,
};
use studiolo_core::ToolId;

/// 仅 Gateway 使用的持久化中继。新实例总是以冻结状态启动。
pub struct DurableRelay {
    store: Store,
    operation: Mutex<()>,
    raw_root: PathBuf,
    guard: Arc<dyn AttemptGuard>,
    policy: Arc<dyn RelayPolicyGuard>,
    clock: Arc<dyn Clock>,
    audit: Arc<dyn AuditLedger>,
    permissions: PermissionLayers,
    transport: Arc<dyn RelayTransport>,
    publisher: Arc<dyn RelayPublisher>,
    _retention: crate::relay_retention::Retention,
}
/// 可信的装配依赖；不存在任何能放行真实网络请求的默认值。
pub struct RelayServices {
    /// 实时的 attempt／fence／取消状态权威来源。
    pub guard: Arc<dyn AttemptGuard>,
    /// 实时的租约、来源分级与生效策略权威来源。
    pub policy: Arc<dyn RelayPolicyGuard>,
    /// 服务端时钟。
    pub clock: Arc<dyn Clock>,
    /// 权威的只追加审计账本。
    pub audit: Arc<dyn AuditLedger>,
    /// 部署、策略与角色三层上限；第四层由 capability 提供。
    pub permissions: PermissionLayers,
    /// 固定供应商的网络传输。
    pub transport: Arc<dyn RelayTransport>,
    /// Broker 清单发布闸门。
    pub publisher: Arc<dyn RelayPublisher>,
}
impl DurableRelay {
    /// 打开仅 Gateway 使用的状态库，以及不参与备份／同步的临时原始数据区。
    pub fn open(
        database: &std::path::Path,
        raw_root: PathBuf,
        services: RelayServices,
    ) -> Result<Self, RelayError> {
        let meta = fs::symlink_metadata(&raw_root).map_err(|_| RelayError::Storage)?;
        if !meta.is_dir() || meta.file_type().is_symlink() {
            return Err(RelayError::Storage);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.mode() & 0o077 != 0 {
                return Err(RelayError::Storage);
            }
        }
        let retention =
            crate::relay_retention::Retention::start(raw_root.clone(), services.clock.clone())?;
        Ok(Self {
            _retention: retention,
            operation: Mutex::new(()),
            store: Store::open(database)?,
            raw_root,
            guard: services.guard,
            policy: services.policy,
            clock: services.clock,
            audit: services.audit,
            permissions: services.permissions,
            transport: services.transport,
            publisher: services.publisher,
        })
    }
    /// 可信的冻结开关；冻结还会在下一个分块处中断正在进行的流式接收。
    pub fn set_frozen(&self, frozen: bool) -> Result<(), RelayError> {
        self.store.freeze(frozen)
    }
    /// 签发一个不透明的随机 nonce，并绑定到一份不可变、已认证的授权（grant）。
    pub fn issue(&self, grant: RelayGrant) -> Result<RelayTicket, RelayError> {
        self.authorize(&grant)?;
        let limits = &grant.limits;
        let worst_cost = u64::from(limits.request_bytes)
            .checked_mul(limits.input_token_microusd)
            .and_then(|input| {
                u64::from(limits.response_tokens)
                    .checked_mul(limits.output_token_microusd)
                    .and_then(|output| input.checked_add(output))
            })
            .ok_or(RelayError::Budget)?;
        if grant.audience != "studiolo.model-relay.v1"
            || grant.snapshot_id.is_empty()
            || grant.policy_hash.is_empty()
            || grant.role.is_empty()
            || grant.adapter.is_empty()
            || grant.model.is_empty()
            || limits.calls == 0
            || limits.calls > 160
            || limits.response_tokens == 0
            || limits.pacing_tools == 0
            || limits.pacing_tools >= limits.wrapup_tools
            || limits.wrapup_tools >= limits.max_tools
            || limits.max_tools > 10000
            || limits.request_bytes == 0
            || limits.request_bytes > 4 * 1024 * 1024
            || limits.response_bytes == 0
            || limits.response_bytes > 16 * 1024 * 1024
            || limits.timeout_seconds == 0
            || limits.timeout_seconds > 300
            || limits.input_token_microusd == 0
            || limits.output_token_microusd == 0
            || limits.reservation_microusd < worst_cost
            || limits.reservation_microusd > limits.budget_microusd
        {
            return Err(RelayError::Denied);
        }
        let mut nonce = [0u8; 32];
        getrandom::fill(&mut nonce).map_err(|_| RelayError::Storage)?;
        let token: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        self.store.grant(&token, &grant)?;
        Ok(RelayTicket::new(token))
    }
    fn authorize(&self, grant: &RelayGrant) -> Result<(), RelayError> {
        let now = self.clock.now_unix_seconds();
        let live = self
            .guard
            .inspect(grant.capability.task_id(), grant.capability.attempt_id())
            .map_err(|_| RelayError::Denied)?;
        if live.is_cancelled()
            || live.fence() != grant.capability.fence()
            || now >= grant.capability.expires_at_unix_seconds()
        {
            return Err(RelayError::Denied);
        }
        let tool = ToolId::from_static("model.relay");
        if !self.permissions.effective(&grant.capability).allows(&tool) {
            return Err(RelayError::Denied);
        }
        if grant.sources.is_empty()
            || grant.sources.iter().any(|s| {
                s.revision.is_empty()
                    || s.sha256.len() != 64
                    || !s.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                    || now >= s.retain_until_unix_seconds
                    || !s.model_egress.allows(grant.provider)
            })
        {
            return Err(RelayError::Egress);
        }
        self.policy.inspect(grant, now)?;
        if self.store.frozen()? {
            return Err(RelayError::Frozen);
        }
        Ok(())
    }
    fn audit(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        phase: AuditPhase,
        decision: &str,
    ) -> Result<(), RelayError> {
        let denial = if decision.starts_with("deny:") {
            format!(":{}", hash(decision.as_bytes()))
        } else {
            String::new()
        };
        let id = AuditEventId::parse(format!(
            "gateway.real.v1:{}:{}:{}{}",
            grant.capability.attempt_id(),
            grant.capability.fence().get(),
            hash(call.as_str().as_bytes()),
            denial
        ))
        .map_err(|_| RelayError::Audit)?;
        let payload = SanitizedPayload::try_from(json!({"call_id":call.as_str(), "fence":grant.capability.fence().get(),
            "decision":decision,"provider":grant.provider,"model":grant.model,"snapshot_id":grant.snapshot_id,
            "policy_hash":grant.policy_hash})).map_err(|_| RelayError::Audit)?;
        let subject = AuditSubject::default()
            .with_task(grant.capability.task_id())
            .with_attempt(grant.capability.attempt_id());
        self.audit
            .append(
                AuditEvent::new(id, phase, "gateway.model_relay", payload).with_subject(subject),
            )
            .map_err(|_| RelayError::Audit)?;
        Ok(())
    }
    /// 中继一个协议已钉死的请求。结果未知的调用保留已扣费用，且永不重发。
    pub fn relay(
        &self,
        ticket: &str,
        call: CallId,
        path: &str,
        body: &[u8],
    ) -> Result<RelayResponse, RelayError> {
        let grant = self.store.authenticate(ticket)?;
        let result = self.relay_authenticated(&grant, &call, path, body);
        if let Err(error) = &result {
            self.audit(&grant, &call, AuditPhase::Outcome, &format!("deny:{error}"))?;
        }
        result
    }
    fn relay_authenticated(
        &self,
        grant: &RelayGrant,
        call: &CallId,
        path: &str,
        body: &[u8],
    ) -> Result<RelayResponse, RelayError> {
        let _operation = self.operation.lock().map_err(|_| RelayError::Storage)?;
        self.authorize(grant)?;
        if path != grant.provider.path() {
            return Err(RelayError::Denied);
        }
        let request = relay_request::normalize(grant, body)?;
        match self.store.reserve(grant, call, &hash(&request))? {
            Admission::Complete {
                raw,
                digest,
                expires,
            } => {
                self.authorize(grant)?;
                if self.clock.now_unix_seconds() >= expires {
                    return Err(RelayError::Expired);
                }
                let path = self.raw_root.join(raw);
                let bytes = fs::read(path).map_err(|_| RelayError::Expired)?;
                if bytes.len() > grant.limits.response_bytes as usize || hash(&bytes) != digest {
                    return Err(RelayError::Storage);
                }
                return Ok(RelayResponse {
                    content_type: "text/event-stream",
                    body: bytes,
                });
            }
            Admission::Execute => {}
        }
        crate::relay_retention::available(
            &self.raw_root,
            grant.limits.response_bytes,
            self.clock.now_unix_seconds(),
        )?;
        let request = crate::relay_tools::inject_notice(grant, &request, self.store.tools(grant)?)?;
        self.audit(grant, call, AuditPhase::Intent, "pending")?;
        self.authorize(grant)?;
        // 在把控制权交给真实传输「之前」，先持久化「结果不确定」边界。
        self.store.mark(grant, call, "indeterminate")?;
        let output = self
            .transport
            .execute(grant, &request, &|| self.authorize(grant));
        let bytes = match output {
            Ok(bytes) => bytes,
            Err(_) => {
                self.audit(grant, call, AuditPhase::Outcome, "indeterminate")?;
                return Err(RelayError::Indeterminate);
            }
        };
        self.authorize(grant)?;
        if bytes.len() > grant.limits.response_bytes as usize {
            return Err(RelayError::Indeterminate);
        }
        crate::relay_transport::validate_terminal(grant.provider, &bytes)?;
        self.store
            .accept_tools(grant, crate::relay_tools::count(&bytes)?)?;
        let digest = hash(&bytes);
        let source_expiry = grant
            .sources
            .iter()
            .map(|s| s.retain_until_unix_seconds)
            .min()
            .ok_or(RelayError::Egress)?;
        // 清理每 30 秒跑一次，故到期时间再提前 30 秒：文件最迟在一小时硬上限前 60 秒被删除。
        let expires = self
            .clock
            .now_unix_seconds()
            .saturating_add(3540)
            .min(grant.capability.expires_at_unix_seconds())
            .min(source_expiry)
            .saturating_sub(30);
        let name = format!(
            "{}-{}-{expires}.raw",
            grant.capability.attempt_id(),
            hash(call.as_str().as_bytes())
        );
        let path = self.raw_root.join(&name);
        let mut file = File::options()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|_| RelayError::Storage)?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| RelayError::Storage)?;
        File::open(&self.raw_root)
            .and_then(|f| f.sync_all())
            .map_err(|_| RelayError::Storage)?;
        // 发布是强制的；发布失败会留下一个不可重放、结果未知的调用。
        self.authorize(grant)?;
        self.publisher.seal(grant, call, &path, &digest)?;
        self.audit(grant, call, AuditPhase::Outcome, "allow")?;
        self.authorize(grant)?;
        self.store.complete(grant, call, &name, &digest, expires)?;
        Ok(RelayResponse {
            content_type: "text/event-stream",
            body: bytes,
        })
    }
}
