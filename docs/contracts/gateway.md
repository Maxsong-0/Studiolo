# Gateway Interface 契约

> Module：`studiolo-gateway`。它集中 capability 校验、活跃 attempt 查询、权限求交、调用
> 幂等与审计顺序。M0 只交付进程内 Interface/内存 Adapter，不执行真实网络、数据库或云
> 模型 I/O。

## 1. M0 Interface

```rust
pub trait Gateway: Send + Sync {
    fn invoke(
        &self,
        capability: &JobCapability,
        call: ToolCall,
    ) -> Result<ToolResult, GatewayError>;

    fn relay(
        &self,
        capability: &JobCapability,
        request: ModelRequest,
    ) -> Result<ModelResponse, GatewayError>;
}
```

`CallId` 是非空、无控制字符的 attempt 内稳定调用身份；自定义反序列化也执行同一校验，
不能绕过 `parse`。`ToolCall` 绑定 call ID、`ToolId` 和 JSON arguments；`ToolResult` 回显 call
ID/tool 并返回 JSON。`ModelRequest/ModelResponse` 绑定
call ID、provider、model 与 prompt/text。参数的 SSRF、SQL、配额和响应验证属于未来真实
tool Adapter；M0 内存映射不提供这些安全声明。

## 2. capability 与活跃状态

M0 `JobCapability` 绑定 task、attempt、fence、允许的 `PermissionSet` 与过期 Unix 秒数。
`PermissionLayers` 固定为 deployment、policy、role 三层；effective permission 是这三层再与
capability 允许集合的四方交集，缺失即拒绝；`PermissionLayers::default()` 是全拒绝。

```rust
pub trait AttemptGuard: Send + Sync {
    fn inspect(
        &self,
        task_id: TaskId,
        attempt_id: AttemptId,
    ) -> Result<AttemptSnapshot, AttemptGuardError>;
}

pub trait Clock: Send + Sync {
    fn now_unix_seconds(&self) -> i64;
}
```

每次调用（包括已有结果的重放）都先通过注入的 `AttemptGuard::inspect(task, attempt)` 读取
当前 fence/cancelled，再检查过期和 effective permission；不得因为命中缓存而绕过撤销。
`relay` 以逻辑工具 `model.relay` 做相同检查。

M0 capability 没有签名、audience、nonce、snapshot 或 policy hash，只能用于受信任进程内
组合与测试，不能接受远程不可信输入。Gateway DTO/guard 本轮留在 gateway crate；代码归属
见 [core-types.md](core-types.md) 与 [decisions.md](decisions.md)。

## 3. 固定顺序与幂等

授权成功的调用顺序：

1. 在 `gateway.tool.v1` / `gateway.model.v1` 版本域中，以 task、attempt、fence 和长度分隔的
   call ID 构造稳定审计 ID；
2. fresh guard → cancelled → fence → expiry → 四层权限求交；
3. 在同一把锁内检查并占用 `(attempt_id, call_id)` 幂等槽；同键调用正在执行时等待其
   完成，再判定同请求重放、异请求冲突或取得已释放的占位；
4. 释放槽锁后完整重做第2步，再返回首次结果、记录独立 conflict outcome 或继续执行。
   等待期间撤权、到期或 guard 故障均优先返回当前拒绝原因；guard 与审计不在槽锁内运行；
5. 新执行者在 AuditLedger `Intent` 追加成功后，才读取配置的内存 tool/model 响应；
6. 以同审计 ID 追加 `Outcome`。**账本一旦写入终局 `Outcome`（`allow` 或 `error`），该
   call ID 即封口**：占位替换为可重放的结果（成功结果或 `ToolUnavailable` /
   `ModelUnavailable` 失败终态），同请求重放经完整重验授权后返回同一结果而不再执行；
   要重试须换新的 call ID。没有任何 `Outcome` 落账的失败（Intent 追加失败、Outcome 追加
   失败、内部状态不可用）才归还占位允许重试。取得占位的执行者持有占位守卫，正常返回、
   提前返回或 panic 展开都归还自己的占位并唤醒等待者；守卫在占位时建立，覆盖二次授权及
   拒绝审计的 panic，同键等待不会因为一次 panic 永久
   挂起；重放与冲突拒绝不删除其他调用的完成结果。
7. 槽内仅保存完整请求的 32 字节 SHA-256 指纹，不保存 arguments/prompt。工具与模型请求
   使用不同版本域；字符串和字段长度分隔，JSON 值区分类型、数组保序、对象键递归排序。
   数字遵守 `serde_json::Value` 相等语义，整数 `1` 与浮点 `1.0` 不同，普通浮点模式下
   `-0.0` 与 `0.0` 相同。已完成结果最多保留 `MAX_RETAINED_RESULTS`（4096）条；同锁的
   完成队列按完成顺序直接逐出最早结果，准入与完成不再为逐出扫描全部历史槽。
   逐出后的 `Evicted` 槽仅留指纹：同请求重放得到独立审计的 `CallResultEvicted`
   拒绝（不重新执行），异请求仍是 `CallConflict`。结果正文随逐出释放；历史 call ID 与
   固定指纹墓碑仍随唯一调用数增长，因此这里只保证正文保留条数与完成队列有界，不宣称
   整张幂等表恒定内存。

实际槽冲突与拒绝审计本身的 `IdempotencyConflict` 在内部区分；后者直接映射错误，
不再追加一条虚构的槽冲突审计。完整复核覆盖 invoke 与 relay 的四种准入结果。
（2026-09-06 T-032：失败终态封口、占位守卫与有界保留为复审修复新增。）

授权拒绝与冲突没有外部动作，因此直接追加带 `decision=deny` 与结构化 reason 的 `Outcome`，
事件 ID 一律为独立的 `{基础 ID}:deny:{reason}`：同一调用因不同原因被多次拒绝时各留一条
记录，且永不与基础 ID 上的 Intent/Outcome 发生幂等冲突。审计 `IdempotencyConflict`
（同一事件 ID 被不同内容复用）在 Gateway 边界映射为 `CallConflict`，其余审计失败为
`AuditUnavailable`。（2026-09-05 T-027 起；此前只在已有完成调用时才换 ID。）

内存 Adapter 未配置响应时，在已写 Intent 后补失败 Outcome，再返回 Tool/ModelUnavailable。
M0 没有真实外部副作用，因而不承诺 raw promotion、持久 call reservation 或
`indeterminate` 恢复。接入真实 Adapter 前必须先补“动作可能已发生”的持久协议，不能直接
复用内存结果缓存冒充 exactly-once。

## 4. 审计与数据边界

- Intent 失败时不得读取/释放配置的 tool/model 结果。
- 审计 subject 绑定 task、attempt、tool；payload 只含 call ID、逻辑 tool、fence、决策和
  必要的 provider/model，不写 arguments、prompt、output 或凭据。
- `SanitizedPayload` 拒绝秘密字段；转换失败与除幂等冲突外的 append 失败在 Gateway 边界
  表现为 `AuditUnavailable` 并失败关闭，幂等冲突表现为 `CallConflict`。
- Gateway 不依赖 TaskEngine crate；当前 attempt 状态只能经 `AttemptGuard` 注入。
- `MemoryGateway` 不写 canonical Vault，也不持有长期 Web/DB/模型凭据。

## 5. M0 错误模式

| `GatewayError` | 含义 |
|---|---|
| `InvalidCallId` | call ID 为空或含控制字符 |
| `AttemptNotActive` | task/attempt 不存在或不活跃 |
| `AttemptGuardUnavailable` | 无法取得 fresh attempt 状态，失败关闭 |
| `Cancelled` | capability 已撤销 |
| `FenceMismatch` | capability fence 不是当前 fence |
| `CapabilityExpired` | 当前时间达到或超过过期时间 |
| `ToolNotAllowed` | 四个允许集合的交集不含逻辑工具 |
| `AuditUnavailable` | 不能完成要求的权威审计 |
| `ToolUnavailable` / `ModelUnavailable` | 内存 Adapter 未配置响应 |
| `CallConflict` | 同 attempt 的同 call ID 绑定不同请求 |
| `StateUnavailable` | 内存状态锁不可用 |

## 6. Adapter 与验收

`MemoryAttemptGuard` 可显式设置 active/cancelled 与 fence；`ManualClock` 控制过期；
`MemoryGateway` 只返回测试配置的 tool 或 provider/model 响应，并使用真实 `AuditLedger` trait。

契约测试至少覆盖：对象安全、四层权限任一层缺失即拒绝、允许 tool 与本地 relay 的
Intent/Outcome、拒绝 outcome、旧 fence/取消/过期、guard 与审计不可用时失败关闭、成功重放
仍先 recheck、同 call ID 异请求冲突、同一调用两次不同原因拒绝各留一条记录并返回真实原因、
确定进入等待后取消/fence变化/精确到期/guard不可用或未知、拒绝审计失败下的占位归属、
并发同键调用只执行一次且共享首次结果、未配置响应的失败 outcome 且同 call ID 重试不再执行、
执行中 panic 释放占位、保留上限逐出后的 `CallResultEvicted`，以及 Cargo 依赖中不存在
TaskEngine。真实 SSRF/SQL/模型外发、秘密管理、raw promotion、预算与 indeterminate 恢复均
延期，不能用这些内存测试宣称通过。

## 7. M6 real relay (T-091)

`DurableRelay` is a separate real-I/O implementation, not a modification of `MemoryGateway` cache semantics. Its required `RelayServices` bind fresh attempt and policy/lease/source guards, authoritative audit, four-layer permissions, a fixed HTTPS transport, and a mandatory Broker publication gate. New instances start frozen. An authenticated scheduler issues opaque nonce tickets; the Runner cannot submit its own role, snapshot, policy, provider, model or price ceiling.

SQLite WAL/FULL persists the budget reservation and call intent before I/O, then marks the uncertainty boundary before handing control to the provider transport. Unknown outcomes block the entire attempt from further real calls, including new call IDs; they are never retried automatically. Completed replays still check current authority. Upstream credentials are redacted non-serializable types held only by `HttpsRelayTransport`; incoming CLI authentication/target headers are never forwarded.

Both Anthropic Messages and OpenAI Responses use bounded SSE ingestion. Responses are released only after a complete terminal event, fresh egress, tool-count reservation, raw retention, Broker seal, audit outcome and durable completion. A raw HTTP request cannot choose a base URL, background/stored conversation or hosted/native subagent tool. Both request and result are checked against the same authenticated source/provider binding. `local_only`/`deny` permit no real provider.

The Unix HTTP entry is `serve_relay_connection`; listener permissions, bounded acceptance and lifecycle belong to the trusted composition root. The exact APIs, retention/spend model, deployment prerequisites and unverified release gates are documented in [M6 implementation boundary](../m6/runner-gateway.md). A permissive test guard/publisher cannot be installed as a production implementation.
