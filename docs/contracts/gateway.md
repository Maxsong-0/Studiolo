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
3. 检查 `(attempt_id, call_id)` 缓存：同请求返回第一次结果；异请求写独立 conflict outcome
   后返回 `CallConflict`，审计失败仍失败关闭；
4. AuditLedger `Intent` 追加成功后，才读取配置的内存 tool/model 响应；
5. 以同审计 ID 追加 `Outcome`；成功后保存幂等结果并返回。

授权拒绝没有外部动作，因此直接追加带 `decision=deny` 与结构化 reason 的 `Outcome`；审计
失败则返回 `AuditUnavailable`。已完成调用后来因取消/旧 fence/过期被拒绝时，使用独立的
recheck 审计 ID，不能与原 outcome 发生幂等冲突。

内存 Adapter 未配置响应时，在已写 Intent 后补失败 Outcome，再返回 Tool/ModelUnavailable。
M0 没有真实外部副作用，因而不承诺 raw promotion、持久 call reservation 或
`indeterminate` 恢复。接入真实 Adapter 前必须先补“动作可能已发生”的持久协议，不能直接
复用内存结果缓存冒充 exactly-once。

## 4. 审计与数据边界

- Intent 失败时不得读取/释放配置的 tool/model 结果。
- 审计 subject 绑定 task、attempt、tool；payload 只含 call ID、逻辑 tool、fence、决策和
  必要的 provider/model，不写 arguments、prompt、output 或凭据。
- `SanitizedPayload` 拒绝秘密字段；转换或 append 失败统一在 Gateway 边界表现为
  `AuditUnavailable` 并失败关闭。
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
仍先 recheck、同 call ID 异请求冲突、未配置响应的失败 outcome，以及 Cargo 依赖中不存在
TaskEngine。真实 SSRF/SQL/模型外发、秘密管理、raw promotion、预算与 indeterminate 恢复均
延期，不能用这些内存测试宣称通过。
