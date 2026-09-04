# AuditLedger Interface 契约

> Module：`studiolo-audit-ledger`。它是权威审计的唯一写入口。M0 交付同步 Interface 与
> `MemoryAuditLedger`；文件持久化、轮换和面向用户的授权查询不属于 M0。

## 1. M0 Interface

```rust
pub trait AuditLedger: Send + Sync {
    fn append(&self, event: AuditEvent) -> Result<AuditRecord, AuditError>;
    fn query(&self, filter: &AuditFilter) -> Result<Vec<AuditRecord>, AuditError>;
}
```

`AuditEvent` 聚合 `AuditEventId`、`AuditPhase::{Intent, Outcome}`、稳定 action、
`AuditSubject` 与 `SanitizedPayload`。`AuditSubject` 可绑定 task、attempt 和 tool；缺失字段只
表示“不参与该维度过滤”，不能当作授权信息。`AuditRecord` 返回一基 offset、服务端时间、
`previous_hash`、`record_hash` 和原事件。

`AuditEventId` 与 `SanitizedPayload` 都使用验证型自定义反序列化；不能通过 serde 绕过非空
ID 或递归秘密字段检查。`AuditRecord::from_parts(...)` 供未来存储 Adapter 重建记录，但该
Adapter 必须先验证 offset 与 hash，构造器本身不是完整性证明。

`query` 的 `AuditFilter` 支持 event、task、attempt、tool、phase、精确 action、
exclusive after-offset 与 limit，结果保持追加顺序；公开 builder 与只读 getter 使 crate 外
Adapter 无需访问私有字段。此 M0 Interface 是进程内测试缝，不提供用户身份
授权；未来 HTTP/CLI 查询必须在调用它之前独立鉴权，不能把 filter 当成权限。

## 2. 追加、幂等与顺序

- 唯一幂等键是 `(event_id, phase)`。
- 相同 key 且整个 `AuditEvent` 相同，`append` 返回第一次产生的 `AuditRecord`，不增加
  offset，也不延长完整性链。
- 相同 key 但 action、subject 或 payload 任一不同，返回
  `AuditError::IdempotencyConflict { event_id, phase }`。
- 受保护动作必须先成功追加 `Intent` 才能发生；完成、拒绝、失败或结果不明用同一 event ID
  的 `Outcome` 表达。
- Interface 不提供 update/delete。`Outcome` 失败时不能把动作报告为已完整审计；调用方保存
  correlation，并以同一事件补记，不能另造一次外部动作。

## 3. 脱敏边界

`SanitizedPayload` 只能通过受检查的构造路径取得。M0 递归检查 JSON object/array，并拒绝
以下大小写不敏感、连字符归一为下划线后的字段名：

```text
token, secret, password, cookie, authorization, api_key,
*_token, *_secret, *_password, *_cookie, *_authorization
```

命中时返回 `AuditError::SecretField { path }`，其中 path 保留对象层级和数组下标。M0 只对
字段名实行 fail-closed 检查，不声称能从任意自由文本值中识别秘密；调用方仍必须只提交
允许的结构化摘要，不能提交原始响应、完整 query、DSN 或异常栈。

## 4. 完整性链

- 每次新 append 产生单调的一基 offset，并把上一条 `record_hash` 写入
  `previous_hash`；首条引用 `SHA256("studiolo-audit-genesis-v1")`。
- `IntegrityHash` 固定为 32 字节 SHA-256 摘要。每条记录 hash 使用
  `studiolo-audit-record-v1\0` domain，并覆盖上一摘要及该条记录的确定性编码。
- `record.previous_hash` 必须等于前一条的 `record_hash`。同一实现一旦加入重启/持久化，编码
  必须在该版本内稳定；M0 不冻结跨实现 hash 向量，也不冻结未来文件 Adapter 的磁盘
  framing、canonical 字节公式、fsync 或轮换格式。
- 相同重放返回原记录，因此时间戳和 hash 也完全不变。

## 5. M0 错误模式

| 错误 | 含义 | 调用方行为 |
|---|---|---|
| `InvalidEventId` | ID 为空或含控制字符 | 修正调用方，不重试 |
| `SecretField { path }` | payload 含禁用秘密字段 | 先脱敏；不得绕过检查 |
| `IdempotencyConflict { event_id, phase }` | 同 key 绑定了不同事件 | 报冲突，不覆盖旧记录 |
| `Serialization` | 事件无法编码入链 | 失败关闭并保留 correlation |
| `Unavailable` | 权威账本/锁不可用 | 受保护动作不得释放 |

`AuditPhase` 是闭合枚举，因此没有字符串型 `invalid_phase`；未知存储 schema、磁盘损坏等错误
等文件 Adapter 落地时再扩展，不能在 M0 文档中冒充已实现。

## 6. 替身与验收

`MemoryAuditLedger` 通过与生产 Adapter 相同的 trait 使用，并实现真实幂等、单调 offset、
SHA-256 前向链和 filter。需要验证调用方失败关闭时，测试注入一个返回 `Unavailable` 的
`AuditLedger` test double；不能让内存实现无条件成功来替代故障场景。

契约测试至少覆盖：

- `AuditEventId` 的空值、控制字符和正常值；
- Intent/Outcome 使用同一事件 ID 形成两个记录；
- 完全相同重放返回原 offset/时间/hash，同 key 异内容返回 `IdempotencyConflict`；
- 首记录、连续记录的 `previous_hash` 与 `record_hash` 链接正确；
- 嵌套 object、array 的秘密字段返回精确 path，安全结构可进入记录；
- 所有 filter、`after_offset`、limit 与追加顺序；
- intent 返回 `Unavailable` 时，Gateway/TaskEngine 的受保护动作调用次数为零；
- 其他 crate 不直接打开 `WORKBENCH_DATA/audit/`。

这些测试只证明 M0 Interface 与内存 Adapter。磁盘耐久性、启动验链、篡改恢复、轮换、OS
身份隔离和面向用户的查询授权，必须等相应 Adapter/入口实现后单独验收。
