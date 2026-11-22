# AuditLedger Interface 契约

> Module：`studiolo-audit-ledger`。它是权威审计的唯一写入口。M0 交付同步 Interface 与
> `MemoryAuditLedger`。T-012 在相同追加/查询 Interface 下新增 `SqliteAuditLedger`；
> T-051 在同一 Interface 下把持久账本按固定边界分段（磁盘格式 v2，见第 8 节）；T-220 让常规操作
> 不再每次从创世重算整条段链（磁盘格式不变，见第 8 节「段链核到链尾」）。
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
| `UnsupportedSchema` | schema/application ID/codec 未知，或库仍是 v1 | 不迁移、不覆盖或重建；v1 须显式升级 |
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

`application_id=0x53544155`；`user_version` 与 `audit_meta.schema_version` 同为磁盘格式版本，
T-012 为 1，T-051 分段格式为 2。仅允许精确的 STRICT schema：

| 对象 | 内容 | 版本 |
|---|---|---|
| `audit_meta` | 单行 id=1；schema_version、codec、last_offset、last_hash | v1/v2 建表语句逐字相同 |
| `audit_events` | 连续正 offset 主键、event_id、phase、规范 record_json | v1/v2 逐字相同 |
| 唯一索引 | `(event_id, phase)`；仅允许 SQLite 生成的对应索引 | v1/v2 |
| `audit_segments` | 每封存段一行：segment_index、last_offset、last_hash、segment_hash | 仅 v2 |

启动与每次 append/query 都在同一数据库事务中执行精确 schema 校验、元信息校验与第 8 节
规定的分段验证：逐条核对行键与记录、连续 offset、previous_hash、重算 hash 与规范编码，
即使 query limit=0 或幂等重放，也不跳过所读范围内的损坏。不接受外加 table/trigger/view、
未知版本、head/记录失配，不自动修复或截短。`quick_check` 与整表行数统计移入显式的
`verify_all()`，不再摊到每次操作上——代价见第 8 节"分段换来的成本"。

查询仅保留符合 filter/limit 的记录；不设 limit 时会返回全部匹配项，未来 HTTP 查询仍须
独立鉴权并限制结果规模。单条规范 record_json 最大 1 MiB；Memory 为兼容既有契约，
不新增此磁盘预算，也不分段。

SQLite 提交前将整条 record 通过实际磁盘解码路径往返，比较记录等值及重新规范编码后的
字节完全一致；无法精确回放则返回 `Serialization`，不插入记录。当前 serde_json 未启用
float_roundtrip，部分普通 f64（例如 `51.248178375505404`）因此会被明确拒绝；调用方宜使用
整数或预定义十进制字符串，不得认为任意 f64 均可持久化。Memory 不执行磁盘往返限制。
读取时先约束投影字段的字节长度；此预算不是 SQLite 页缓存或整个进程内存的全局上限。

### 提交与耐久性

append 使用 BEGIN IMMEDIATE，先做第 8 节的分段验证，再在同一 transaction 中检查精确重放、
插入记录、必要时封存写满的段并更新 head；多实例依靠 SQLite 写事务串行分配 offset。相同 key 异内容冲突，不产生
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
T-220 起有一处窄的例外：一个已打开的实例核过的封存段（包括它自己刚封存、提交成功的那一段）
只能往后长，别的连接把账本退回到更早、自身自洽的一版（封存段变少或被换掉），这个实例的下一次操作失败关闭；新打开的实例
没见过退回之前的样子，仍然分不出来，打开段内的回退也分不出来。这不是外部可信锚。

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

## 8. T-051 固定分段与可伸缩性（磁盘格式 v2）

### 段边界与段摘要

账本按 offset 固定分段，每段 `AUDIT_SEGMENT_RECORDS = 128` 条：段号为 `(offset-1)/128`，
第 `i` 段覆盖 `[128i+1, 128(i+1)]`。段大小是磁盘格式的一部分，改动必须升版本。只有写满的
段才封存，并在写入段内最后一条记录的**同一事务**里写下一行 `audit_segments`；尾部未写满
的段称为**打开段**，没有摘要行。

```text
段链创世        = SHA256("studiolo-audit-segment-genesis-v1")
segment_hash(i) = SHA256( "studiolo-audit-segment-v1\0"
                          ‖ segment_hash(i-1) 的原始 32 字节
                          ‖ canonical([i, first_offset, last_offset,
                                       previous_hash, last_hash]) )
```

`previous_hash` 是段内第一条记录的前驱哈希（即上一段的 `last_hash`，首段为记录链创世），
`last_hash` 是段内最后一条记录的 `record_hash`。磁盘上只存 `last_hash` 与 `segment_hash`，
段边界与 `previous_hash` 全部由段号、固定段大小和前一行重算，**没有可以单独改写而不破链的
冗余字段**。`canonical` 沿用第 4 节的 `studiolo-audit-json-v1`；段 domain 与记录 domain
不同，两类哈希不可互换。

### 每次操作验什么

`open`/`open_existing`/`append`/`query`/`AuditReadView::scan_page`/`verify_segment` 在同一事务快照内一律执行：

1. 精确 schema 校验与 `audit_meta` 元信息校验；
2. **段链核到链尾**（下一小节）：段号必须自 0 连续、段边界必须落在固定倍数上、每行
   `segment_hash` 必须由前一行重算得到，最后一行必须与打开段首尾相接；
3. `min(offset)` / `max(offset)` 必须正好是 `1` 与 `last_offset`（分两句取，各走一次 rowid 主键，
   O(log N)；写进同一句 SQLite 会改成扫整个唯一索引，见 BUG-042）；
4. **走读整个打开段**并重算记录链，链尾必须等于 `audit_meta.last_hash`。

在此之上，`append` 命中幂等键时连带走读该记录所在的整段；`query` 对每个出现候选行的段
走读整段，再在已验证记录上应用完整过滤。**任何返回给调用方的记录都被其所在段的记录链
重算过，并经段链锚定到 head**，不存在取出来直接返回的路径。读到某个封存段时，这一段的
摘要行若这次快照里没重读过（沿用本连接早先核过的那份），先重读并逐字段比对：读到哪段，
那段的记录和摘要行都当场核过。

### 段链核到链尾（T-220）

每个 `SqliteAuditLedger`／`AuditReadView` 实例一条连接，这条连接把它核过的段摘要留在内存
（每段约 152 字节，70 万条约 830 KiB；T-220 之前每次操作也要临时建这么一份）。核到链尾有两种做法：

| 情形 | 怎么核 | 段链部分的成本 |
|---|---|---|
| 打开、`verify_all()`、`segments()`、没有缓存的一次性连接（封存核对、升级） | 从创世重算整条段链 | O(段数) |
| 常规操作，且 `PRAGMA data_version` 与上次核链时相同 | 重读链尾那一行逐字段比对，再按公式核其后新封存的行 | O(1 + 新封存段数) |
| 常规操作，`data_version` 变了 | 同第一行，从创世重算 | O(段数) |

判据来自 SQLite 的承诺：这期间只要有别的连接向该库提交过改动，同一条连接两次读到的 `data_version`
就一定不同；本连接自己的提交不改变它（别的原因也可能让它变，那只是多全算一次）。值没变，库里只多了本连接自己写的记录、封存行和 head，
已核过的摘要行一行没动，续核即可。值变了（另一个实例追加、有人经别的连接改库，或 SQLite 因别的连接的检查点重置了 WAL），
不知道动了哪里，就全算。每次全算都要求新链以本连接核过的那段为前缀，否则失败关闭（即上面第 7 节
那处「退回旧版」的例外）。锚定失败、读不到或中途出错，记下的 `data_version` 就作废，下一次一律全算；
已核过的段留着，当作下一次全算必须满足的前缀（其中可能有这次按段链公式核过、还没来得及锚定到 head
的行，同样读自已提交的快照），所以失败之后只会更严，直到重新打开。缓存只放已提交的行：从已提交快照里
读出并核过的行，以及本实例自己封存、提交成功之后的那一段；事务里刚写下、还没提交的封存行不放，事务
回滚不会让缓存多出库里没有的段。读的顺序与 T-220 之前一致：同一快照里先读 head（`audit_meta` 的版本或
codec 不对就报 `UnsupportedSchema`），再核段链。

人侧生产账本平时只有人侧服务一条长连接在写。管理命令（`vault-admin` 初始化、`auth-admin`、维护命令）
各自打开同一本账本、需要时追加；新根恢复 `studiolo-restore` 在恢复和最终切换时打开新根的
人侧账本、追加撤权记录（那时人侧服务还没在这本上跑，切换之后它才是生产账本）；门铃（`deploy/doorbell/`，ADR-033 候选，尚未装进生产）装上之后由定时器
每分钟拉起一次，也打开同一本账本，启用时、每次投递前后和事件过期时追加。它们每次打开都从创世全算一次，
追加过之后人侧服务的下一次操作全算一次，都与 T-220 之前相同；其余常规操作走第二行。一个进程读另一个进程正在写的账本（如人侧读 agent 账本的
读窗口），对方每提交一次，下一次读就全算一次，与 T-220 之前相同，不更慢。

### 分段换来的成本，以及换掉的是什么

本实例独自读写时，每次操作的验证量是 `O(1 + 新封存段 + 打开段记录数 + 读到的段 × 段大小)`，
与账本总量无关（另有 SQLite 自身 B 树查找的 O(log N)）。把话说准：

- **只在「别的连接没提交过」时成立。** 打开时、`verify_all()`／`segments()`、别的连接提交过之后的
  第一次操作，段链部分仍是 O(段数)：每段一行定长摘要（约 160 字节、一次 SHA-256）。T-051 时每次操作
  都付这一项，70 万条约 5470 行；T-220 之前 108 上实测单次追加随总量线性涨（见下文「T-220 量测」）。
- 段大小取舍（T-051）：`S × 记录成本 + (N/S) × 摘要成本` 在 `S ≈ √N` 附近最优；T-220 之后常规操作
  不再付 `(N/S)` 那一项，128 只决定「读到一段」与打开段的代价。段大小是磁盘格式的一部分，没有改。
- **换掉的是检测时机，不是可验证性。** 已封存段**内部**的改写（改段内记录、删段内记录）
  不会被一次与该段无关的 append 发现，因为那次操作根本没读它——**读不到的数据无法被检测，
  这是信息论下界，不是实现缺陷**；要每次操作都发现任意位置的篡改，就必须每次读完整个账本，
  那正是本片要去掉的成本。段链保证这类改写一定破坏该段的 `last_hash` 与 `segment_hash`，
  因此只要该段被读到（`verify_all`、`verify_segment` 或任何命中该段的查询）就必然暴露。
- 下列情形仍**每次操作当场失败关闭**：经别的 SQLite 连接提交的摘要行被删或重排、段号不连续、
  段边界被挪、摘要被替换，本实例核过的封存段变少或被换掉；经 SQLite 提交的链尾那一行被改（链尾每次
  都重读）；`head` 与段链或打开段不符、账本首尾 offset 越界、打开段内任何增删改。
- **T-220 带来的检测时机变化，只有一处**：`data_version` 只反映经 SQLite 的别的连接提交的改动。
  **不经 SQLite、直接改文件字节**的改动，若落在本连接已核过、这次又不重读的摘要行上，要等读到该段、
  `verify_all()`、`segments()`、别的连接提交之后的那次全算或重新打开时才发现；就算落在每次都重读的
  链尾那一行上，也可能被本连接页缓存里的旧页挡住、不当场发现。T-220 之前每次全算读的也可能是本连接
  页缓存里的旧页，同样未必当场发现，因此这里不宣称以前就能。
- 未做「只验最近 N 条」「抽样验证」这类削弱：段链要么从创世全算，要么只在能证明没被别的连接动过
  的前缀上续核；读到的段全段重算并重读摘要行；全链随时可由 `verify_all()` 完整重放。

### 全量与按段验证

| 入口 | 覆盖 | 成本 |
|---|---|---|
| `verify_all()` | `quick_check`、整表行数、从创世重算的段链（并核本实例的前缀）、**每一段**的记录链与打开段 | O(账本总量) |
| `verify_segment(i)` | 段链（续核或全算）+ 打开段 + 第 `i` 段记录链与摘要行；`i` 超出封存范围返回 `Ok(None)` | O(1 + 段大小)；别的连接提交过时 O(段数 + 段大小) |
| `segments()` | 从创世重算的段链（并核本实例的前缀），不走读段内记录 | O(段数) |
| `last_verification_cost()` | 最近一次成功操作实际读入并重算的摘要行数（含续核时重读的链尾、读到某段时重读的摘要行）与记录条数 | O(1) |

`verify_all()` 是本 Adapter 对"账本完整"的完整定义，也是已封存段内部改写的兜底。
**它不由 Adapter 自行调度**：部署方必须把它接成定期巡检或恢复前置步骤；没有接线之前不得
声称账本被定期完整验证过。备份封存（`FrozenAudit`，BUG-045，2026-11-22）：冻结窗口里只验 schema 和首尾锚点
（和每次读写同一成本），把账本定格；整本 `verify_all` 在放开之后、在导出的副本上做，锚点必须等于冻结时那一个，
验不过不出票。中间一条记录被改坏时冻结照常成功、导出失败。`verify_segment` 支持中断后按段续验，重启后可从任意段号继续。
`last_verification_cost()` 只是成本证据，数值小不代表未读部分正确。

### T-220 量测（108，发布构建，2026-11-22）

`cargo --locked test --release -p studiolo-audit-ledger --lib measure_ -- --ignored --nocapture`（`tests/support/scale.rs`）：
三本合成账本用生产同一套编码一次写成，`open_existing` 核过才量；三本轮流量 8 轮、每轮每本追加 25 条、尾部查询 10 次，
记录是人侧最常见的任务对账事件。三列分别是 main `9defcbb` 原样、只加首尾边界两句分开（`5a0cf9c`）、再加段链缓存（T-220）。

| 账本条数 | 单次追加（毫秒） | 每次追加读的段摘要行 | 尾部查询（毫秒） |
|---|---|---|---|
| 1 万 | 2.16 → 1.50 → **1.18** | 78 → 78 → **1.00** | 2.41 → 1.76 → **1.40** |
| 10 万 | 9.21 → 3.93 → **1.24** | 782 → 782 → **1.00** | 9.25 → 4.01 → **1.40** |
| 70 万 | 63.72 → 19.06 → **1.20** | 5469 → 5469 → **1.01** | 63.17 → 18.92 → **1.39** |
| 10 万 ÷ 1 万 | 4.26 → 2.62 → **1.05** | | |
| 70 万 ÷ 1 万 | 29.46 → 12.73 → **1.02** | | |

打开（从创世全算一遍）70 万条时 67.7 → 20.6 → 18.2 毫秒，仍随段数涨，只在启动时付一次。108 上同时有别的项目的任务在跑，
绝对值有起伏，三本轮流量是为了让比值不被某一段时间的负载带偏。没在 169 上量。

### 查询的索引使用

候选行查询只把落在真实列上的条件下推：`offset` 走 rowid 主键（`offset > ?` 恒成立，计划恒为
`SEARCH ... USING INTEGER PRIMARY KEY`），`event_id`（可再带 `phase`）走 `UNIQUE(event_id, phase)`
自动索引。`task`/`attempt`/`tool`/`action` 只存在于规范 JSON 正文里，**没有索引**，这类过滤仍要
遍历候选区间，`limit` 也不下推；这是明确代价，不假称全部条件都走索引。幂等重放同样用唯一
索引定位，不随账本增长。测试用被测连接上的 `EXPLAIN QUERY PLAN` 核对上述计划。

### v1 → v2 显式迁移

`open`/`open_existing` 只接受 v2：v1 库返回 `UnsupportedSchema`，**不补表、不改版本、不重写
任何记录**。`SqliteAuditLedger::upgrade_existing(dir)` 是唯一升级入口，只接受恰好是 v1 的既有库
（已是 v2 同样拒绝），在一个 IMMEDIATE 事务里先 `quick_check`、整表统计与全链验证，再建
`audit_segments`、回填摘要、置版本为 2；任一步不过即整体回滚，磁盘保持原状。**记录
`record_json` 一个字节都不改**：段摘要是由已验证记录链重算出的派生数据，因此升级前后重算
结果逐字节一致。升级成本与账本总量成正比，是一次性人工运维动作。

### 新增测试覆盖

段边界前后各一条的验链与段间链接；封存段中间记录被改写后 `verify_all`、`verify_segment` 与
任何读到该段的查询都失败关闭；摘要行被删/重排/改写/凭空插入时每次操作当场拒绝；摘要与内容
不符（段尾记录被删）在走读该段时失败关闭；重启后逐段续验并单独指出坏段；重启、追加与尾部
查询的验证规模有界，而 `verify_all` 确实逐条读完全部记录（数值由 `last_verification_cost()` 与
`verify_all()` 返回值当场断言）；全部 filter 形态的 `EXPLAIN QUERY PLAN` 不出现整表 `SCAN`；
v1 被拒、显式升级、升级失败整体回滚、升级后回执与段摘要同原追加回执一致。

T-220 新增（`tests/sqlite_chain_cost.rs`）：2 个与 9 个封存段的账本上，追加（含写满封存一段）、
尾部查询、幂等重放、按段续验读的段摘要行数逐步相同且不超过 2（改前分别是段数与段数 + 1，先红）；
另一个实例追加之后，下一次操作读的行数等于段数（全算），再下一次回到不超过 2；打开之后经别的连接
改首段、中间段、链尾摘要行或凭空插一行，下一次追加和查询都失败关闭；别的连接把三段的账本退回到
自洽的两段，正在用的实例失败关闭、新打开的实例照常打开；退回两段后再经别的实例写上另一批事件、长回同样
三个封存段（段数没变、第 2 段换了内容），正在用的实例也失败关闭；退回之后 `verify_segment(0)`、
`segments()` 也失败关闭；本实例自己刚封存、提交成功的那一段被别的连接连同最后一条记录抹掉，下一次
追加和查询失败关闭；只读的读窗口读过最新一页之后，别的连接改这一页读不到的首段摘要行（只有按 `data_version` 全算才发现得了）
或退回到自洽的两段，再读一页失败关闭；`audit_meta` 标着别的版本、段链又少了一段，打开和追加都先报 `UnsupportedSchema`。`tests/support/chain.rs`（crate 内私有）用被测连接
自己改库来模拟 `data_version` 看不见的改动：链尾那一行被删，或 `segment_hash`、`last_offset`、`last_hash` 任一列
被单独改掉，下一次追加失败关闭；中间一行被改，常规追加照常成功（上文写明的检测时机变化），读到那一段、
`segments()`、`verify_all()` 和重新打开都失败关闭；写满封存的那次追加在封存行写下之后失败、整个事务回滚，
重试照常只核链尾并真正封存这一段（缓存里没有回滚掉的段）。
以上每条判据都做过变异检查（去掉 `data_version` 判据、前缀比对、把前缀比对换成只比段数、读段时的摘要行比对、
链尾比对、链尾只比 `segment_hash` 不比 `last_hash`、链尾缺失检查、续核跳过链尾、把事务里刚写的封存行放进缓存、
提交成功后不把刚封存的段放进缓存、读窗口不看 `data_version` 总走续核，各自让对应用例变红）。T-220 的用例、变异检查和量测都在 108（x86_64 Linux）
上跑，合 main 前过 ci 套。量测 `tests/support/scale.rs`（不进门禁，数字见上文「T-220 量测」）。

**未覆盖，不得当作已完成**：真实断电与恢复演练；`verify_all()` 的定期巡检接线（属组合根/
可观测性，本片只提供入口）；多层塔式摘要（没做；T-220 之后本实例独自读写时常规操作与段数无关，但打开、`verify_all()`／`segments()`、
别的连接提交之后的那一次和跨进程读窗口仍是 O(段数)，要去掉这部分仍需塔式摘要）；
不经 SQLite 直接改文件字节的篡改（没有用例，见上文「检测时机变化」）；读 `data_version` 本身出错的那一步
（造不出这种故障，靠代码结构保证：两个锚定入口先作废记下的 `data_version` 再读它）；`task`/`attempt`/`tool`/`action` 的索引化；
本片仅在 macOS 上跑过 `cargo test -p studiolo-audit-ledger`，Linux 原生与全 workspace 回归
由主线统一验收。
