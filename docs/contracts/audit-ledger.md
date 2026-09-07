# AuditLedger Interface 契约

> Module：`studiolo-audit-ledger`。它是权威审计的唯一写入口。M0 交付同步 Interface 与
> `MemoryAuditLedger`。T-012 在相同追加/查询 Interface 下新增 `SqliteAuditLedger`；
> 此文描述实现契约，不代表完整 M2、真实断电演练、轮换或用户授权查询已完成。

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

T-013 增加 `AuditRecord::verify_integrity()`：复用本 Module 的固定编码，校验单条
回执的偏移、首条创世前驱与内容哈希。Auth 用它检查已持久回执，不复制编码公式。
它不访问账本、不证明来源、不中继用户授权，也不验证相邻链或抵御整体重写。

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

`SanitizedPayload` 只能通过受检查的构造路径取得。M0 递归检查 JSON object/array，把字段名
按 `-`、`_`、`.`、空白与驼峰边界切成小写词（`apiKey`、`x-api-key`、`API_KEY` 归一为同一组词），
并拒绝**最后一个词**命中以下词汇、或**最后两个词**命中以下词组的字段名：

```text
词汇：token, secret, password, passwd, pwd, passphrase, cookie, authorization, bearer,
      credential, credentials, apikey, privatekey, secretkey, accesskey, otp, totp, seed
词组：api key, private key, secret key, access key, signing key, session key,
      recovery code(s)
```

末词的常见英文复数按单数判定（`tokens`、`cookies`、`api_keys` 同样拒绝）；末词若是凭据本体后缀
（hex / value / material / plain / plaintext / raw / b64 / base64 / bytes / text / str / string）
则退一个词再判定，因此 `seed_hex`、`token_value`、`api_key_b64` 也拒绝（2026-09-06 T-032）。
其余情况只看末词，所以 `token_count`、`session_id`、`seed_ref`、`token_digest`、单独的 `key` 或 `code` 不会被误伤；
`refreshToken`、`user.pwd`、`PRIVATE_KEY` 会被拒绝。（2026-09-05 T-027 由前缀/后缀字符串
规则改为词法规则；后续存储 Adapter 若把该函数迁到独立模块，规则必须原样保留。）

T-029整合时词法实现移入`src/payload.rs`，保留T-012的迭代深度校验和拒绝释放。
检查也适用于反序列化：升级前T-012 v1记录若含新禁止的`apiKey`等字段，即使原链
自洽，回执JSON也会拒绝；SQLite全表检查使open/query/append返回`CorruptStore`，
不会跳过、删改或自动迁移该旧记录。未命中的旧v1记录仍按原codec校验和精确重放。
此处不扩大到T-012之前未冻结编码的任意M0外存回执；当前Auth自产审计字段未命中新词表。

命中时返回 `AuditError::SecretField { path }`，其中 path 保留对象层级和数组下标。M0 只对
字段名实行 fail-closed 检查，不声称能从任意自由文本值中识别秘密；调用方仍必须只提交
允许的结构化摘要，不能提交原始响应、完整 query、DSN 或异常栈。

T-012 新增两种 Adapter 共同遵守的深度限制：payload 根深度为 0，每经过一个 object
成员或 array 元素增加 1，任一节点深度不得超过 64。先迭代检查再允许递归编码；拒绝时
也迭代释放输入，避免程序构造的超深 Value 在错误路径发生递归栈耗尽。超过边界返回
`PayloadTooDeep`，不产生记录或推进 offset。这是相对旧 M0 的明确输入限制，不是旧持久
数据迁移；JSON 文本仍须通过 serde_json 自身解析预算，不能通过反序列化绕过检查。

## 4. 完整性链

- 每次新 append 产生单调的一基 offset，并把上一条 `record_hash` 写入
  `previous_hash`；首条引用 `SHA256("studiolo-audit-genesis-v1")`。
- `IntegrityHash` 固定为 32 字节 SHA-256 摘要。每条记录 hash 使用
  `studiolo-audit-record-v1\0` domain，并覆盖上一摘要及该条记录的确定性编码。
- `record.previous_hash` 必须等于前一条的 `record_hash`。同一实现一旦加入重启/持久化，编码
  必须在该版本内稳定；M0 不冻结跨实现 hash 向量，也不冻结未来文件 Adapter 的磁盘
  framing、canonical 字节公式、fsync 或轮换格式。
- 相同重放返回原记录，因此时间戳和 hash 也完全不变。

T-012 为新记录冻结 `studiolo-audit-json-v1`：所有 JSON object 键递归按 Rust 字符串顺序
排序，array 保持顺序，采用 serde_json 的紧凑 UTF-8 编码；不称为 RFC 8785/JCS。
hash 输入为 domain `studiolo-audit-record-v1\0`、上一条 hash 的原始 32 字节、以及规范 JSON
数组 `[offset, recorded_at, previous_hash, event]` 的拼接。时间使用 jiff Timestamp 的
RFC 3339 serde 表示，hash 在 JSON 中为 32 个字节的数组。磁盘 `record_json` 本身也须为
规范 JSON，额外字段、重复字段或额外空白不能当作同一磁盘记录接纳。

`src/codec/tests.rs` 锁定固定时间、中文/换行/嵌套对象向量；期望 hash 为
`4d1b38103973dd2df65baf9398e5ae4ac674fd69b96c56fccfa0bf496dfe8b3e`。
向量期望值由独立 Node 标准库 SHA-256 离线计算，不来自被测 Rust 实现。

**兼容范围**：旧 M0 没有持久 Adapter，也未冻结事件 object 的编码键顺序。T-012 的
Memory/SQLite 新记录共享 v1 编码；事件、精确重放、filter 和 public imports 保持兼容，
但不保证重算结果等于旧内存实现的 hash。不存在把旧 SQLite 记录默默重写成新编码的路径；
未知 codec/schema 一律拒绝，未来升级必须显式定义迁移。

## 5. 错误模式（M0 兼容与 T-012 扩展）

| 错误 | 含义 | 调用方行为 |
|---|---|---|
| `InvalidEventId` | ID 为空或含控制字符 | 修正调用方，不重试 |
| `SecretField { path }` | payload 含禁用秘密字段 | 先脱敏；不得绕过检查 |
| `IdempotencyConflict { event_id, phase }` | 同 key 绑定了不同事件 | 报冲突，不覆盖旧记录 |
| `Serialization` | 事件无法编码入链 | 失败关闭并保留 correlation |
| `Unavailable` | 权威账本/锁不可用 | 受保护动作不得释放 |
| `UnsafeStorage` | 目录、权限、链接或对象身份不安全 | 修正部署目标，不自动修复权限 |
| `UnsupportedSchema` | schema/application ID/codec 未知 | 不迁移、不覆盖或重建 |
| `CorruptStore` | schema、记录、offset、head 或链校验失配 | 隔离并走人工恢复 |
| `UnsupportedPlatform` | 非 Unix 平台 | 不放宽文件约束 |
| `RecordTooLarge` | 规范记录超过 1 MiB | 调用者缩减允许的结构化摘要 |
| `PayloadTooDeep` | payload 节点深度超过 64 | 调用者扁平化结构化摘要 |

`AuditPhase` 是闭合枚举，因此没有字符串型 `invalid_phase`。SQLite 的忙、读写或底层
数据库错误映射为 `Unavailable`，不携带环境路径、SQL 参数、事件正文或底层异常文本。

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

以上历史测试只证明 M0 Interface 与内存 Adapter。新增 SQLite 契约见下节，不能把它们
当作已经完成真实断电、灾难恢复、轮换、OS 身份隔离或面向用户的查询授权验收。

## 7. T-012 私有本地 SQLite Adapter

```rust
SqliteAuditLedger::open(private_directory: &Path) -> Result<Self, AuditError>
```

- 只支持 Unix。调用者必须指定**部署批准、位于 Vault 外**、预先存在的私有本地目录；
  固定文件名 `audit.sqlite3`，没有任意数据库名、自动发现或自动创建目录的入口。
- 目录必须绝对路径、0700；各路径分量不得为符号链接。DB、`-wal`、`-shm`、`-journal`
  必须为 0600、与目录同所有者、单硬链的普通文件。新 DB 使用 create_new/0600，SQLite
  使用 NOFOLLOW；已有对象不自动 chmod、不删除或覆盖。Unix 外明确 UnsupportedPlatform。
- 目录及 DB 的 dev/inode 在打开后保留并反复核对。**父目录和同 UID 进程必须可信**，
  SQLite 会按路径管理侧文件，因此这些检查不是目录句柄沙箱，也不能抵御检查间隙中
  同权限进程的恶意改名。目录是否处于 Vault 外由组合根批准，Adapter 不遍历真实 Vault。
- 根目录所在文件系统须支持本地 SQLite 锁、WAL 与同步语义；不支持把此承诺用于 NFS、
  SMB、同步盘或其它未验证的远程文件系统。Linux/macOS 生产保证以对应原生验收为准。
- 初次创建仅由成功 create_new 的实例初始化；若在初始化中途失败，空库/半初始化库保留，
  下次失败关闭，须人工隔离后恢复。并发首次初始化的竞争方可能失败；初始化完成后多个
  实例可安全共同追加。不得把存在的空文件当成“新空账本”自动清空重建。

### 磁盘格式与完整性

`application_id=0x53544155`、`user_version=1`。仅允许精确的 STRICT schema：

| 对象 | 内容 |
|---|---|
| `audit_meta` | 单行 id=1；schema_version、codec、last_offset、last_hash |
| `audit_events` | 连续正 offset 主键、event_id、phase、规范 record_json |
| 唯一索引 | `(event_id, phase)`；仅允许 SQLite 生成的对应索引 |

启动与每次 append/query 都在同一数据库事务中执行精确 schema 校验、quick_check、元信息
校验及按 offset 的全链扫描。逐条核对行键与记录、连续 offset、previous_hash、重算 hash、
规范编码和最后 head；即使 query limit=0 或幂等重放，也不能跳过链上其它损坏。
不接受外加 table/trigger/view、未知版本、head/记录失配，不自动修复或截短。

首版每次操作为 O(完整账本字节数)，扫描流式处理，查询仅保留符合 filter/limit 的记录；
不设 limit 时会返回全部匹配项，未来 HTTP 查询仍须独立鉴权并限制结果规模。单条规范
record_json 最大 1 MiB；Memory 为兼容既有契约，不新增此磁盘预算。

SQLite 提交前将整条 record 通过实际磁盘解码路径往返，比较记录等值及重新规范编码后的
字节完全一致；无法精确回放则返回 `Serialization`，不插入记录。当前 serde_json 未启用
float_roundtrip，部分普通 f64（例如 `51.248178375505404`）因此会被明确拒绝；调用方宜使用
整数或预定义十进制字符串，不得认为任意 f64 均可持久化。Memory 不执行磁盘往返限制。
读取时先约束投影字段的字节长度；此预算不是 SQLite 页缓存或整个进程内存的全局上限。

### 提交与耐久性

append 使用 BEGIN IMMEDIATE，先完整验证，再在一个 transaction 中检查精确重放、插入
记录并更新 head；多实例依靠 SQLite 写事务串行分配 offset。相同 key 异内容冲突，不产生
第二条记录；commit 成功后才返回新 receipt。query 使用同一读事务的完整快照，不混合新旧代。

配置并核读 `journal_mode=WAL`、`synchronous=FULL`、`foreign_keys=ON`、`trusted_schema=OFF`、
`busy_timeout=5000`、`fullfsync=ON`、`checkpoint_fullfsync=ON`，避免未知 pragma 被静默忽略。
WAL+FULL 的同步承诺依赖 SQLite、操作系统与存储设备正确执行同步请求；fullfsync 在 macOS
请求更强的 F_FULLFSYNC，在其它 Unix 不声称额外硬件保证。没有真实拔电实验时不得称已通过
断电验收。忙满 5 秒或 I/O/commit 失败即返回错误；commit 结果不明时必须使用原 event/phase
重试，不能据错误另造一次受保护外部动作。

哈希链能检测非一致篡改、缺行和未同步更新的删尾；**不能阻止或证明发现同权限管理员
同时改写 DB/head/整条链，或恢复整个旧数据库快照**。没有外部可信锚、不可变远端和独立
身份时，不能称为管理员不可篡改账本。此片不实现归档、轮换、备份协议或尾部自动恢复。

新增测试覆盖重启精确重放、冲突、多实例并发、全部过滤器、schema/记录/head 篡改、忙锁、
真实 SQLite `max_page_count` 触发的 `SQLITE_FULL` 与恢复后同键重试、64/65/130 层深度边界、
超深拒绝释放、浮点回放失真、权限/符号链接/硬链接/特殊节点/对象替换、超大记录及连接 pragma。
容量失败测试不是 OS 磁盘填满、commit I/O 故障注入或断电实验。未运行的测试必须标为
“待主线验证”；真实断电、Linux/macOS 原生和部署验收与契约测试分别记账。

2026-09-04 T-012 主线验证已通过：审计29项（原7项＋新增22项），含真实SQLITE_FULL与
黄金向量；全部workspace macOS246 / Linux aarch64 254，格式与严格Clippy通过。
首轮沙箱阻止Unix socket夹具，升级授权后完整复跑成功，未跳过测试。
本次Linux为固定Docker原生aarch64，不是部署服务器x86_64验收；未做真实断电或恢复演练。
完整分项与前端回归见 [M2状态](../m2/README.md)，后续接线不得把本片通过误称完整M2完成。
