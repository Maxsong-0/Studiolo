# 跨项目黑板汇总契约（`GET /api/v1/board/overview`）

T-130，2026-09-22。驾驶舱与协作拓扑一次取全的只读汇总，设计来源是
[T-130 设计规范](../design/t130-design-system.md)第 10 节。实现位置：

- 接口：`crates/local-server/src/board/overview.rs`（路由挂在 `board/http.rs`）。
- 引擎只读清单：`PersistentTaskEngine::board_list_all`（`crates/task-engine/src/board.rs`），
  与 `board_list` 同一投影，`board_list` 现在就是它按项目过滤的结果。
- 前端：封闭解码 `web/src/boardOverviewModel.ts`，取数 `web/src/boardOverviewApi.ts`。

## 1. 请求与状态码

`GET /api/v1/board/overview?window_hours=N`

- 查询参数只有 `window_hours`：十进制整数 1–168，缺省 24。其他参数（大小写不同也算）、
  同名参数出现两次、非数字、小数、负数、空值、越界，一律 **400**。
- 与 `/api/v1/board` 同一套闸门：挂在会话门禁内，未登录 **401**；本机没有装配黑板平面
  **404**（查询串先按字段反序列化：未知参数、非数字等解析失败在未装配时也先得 400；
  0、169 这类能解析的越界值在装配检查之后才判，未装配时得 404）；黑板许可（4 个，与机器入口、
  回放、时间线共用）取不到立即 **429**，不排队；控制存储身份、策略、审计、实例链不可用，
  或数据本身不自洽（例如回执状态不在四个取值里、审计偏移不连续）**503**，正文为空。
- 只读：不写审计、不写控制库、不触发任务页投影。

## 2. 响应字段

字段名 snake_case，每层字段集合固定，多一个或少一个都算无效（前端按此封闭解码）。

| 顶层字段 | 类型 | 含义 |
|---|---|---|
| `generated_at` | RFC3339 UTC | 服务端开始汇总的时刻，恒等于 `window_to` |
| `window_from` / `window_to` | RFC3339 UTC | 事件窗口，闭区间；跨度恰为 `window_hours` 小时 |
| `policy_hash` | 64 位十六进制 | 当前 effective 策略哈希，与 `/api/v1/board` 比较 `uses_old_policy` 用的是同一个 |
| `items` / `items_truncated` | 数组 / 布尔 | 工作项汇总，见下表与第 3 节 |
| `events` / `events_truncated` | 数组 / 布尔 | 任务事件，见下表与第 3 节 |

| 工作项字段 | 类型 | 来源（`BoardView`） |
|---|---|---|
| `id` | ULID | `task.task_id` |
| `title` | 字符串 | `task.spec.title`；缺失或全空白（Unicode 空白再加 U+FEFF，浏览器 `trim()` 会去掉它）时回落为 `id`，因此两端都不会读到空白标题 |
| `project` | 路径 | `task.spec.project`（创建时已按 `VaultPath` 校验） |
| `status` | 14 个任务状态之一 | `task.status` |
| `parent` | ULID 或 null | `board.definition.parent` |
| `adapter` | `stub`/`consumer`/`claude`/`codex` | `task.spec.adapter` |
| `holder` | 实例 id 或 null | 当前或最近一次尝试的 `worker_id`（同 `BoardView.holder`） |
| `holder_lineage` | `[{id, agent_type}]` | 见第 6 节；无 `holder` 时为 `[]` |
| `paths` | 路径数组 | `board.definition.files_owned`，原样（目录前缀，不是 glob） |
| `attempt` | 对象或 null | `{phase, worker, active, lease_until_ms}`：当前或最近一次尝试；`active` 为真当且仅当它就是 `task.active_attempt` |
| `progress` | 对象或 null | `{tools, edits, last_progress_ms}`：该尝试的计数（`tool_calls`、`edits`、最后进展毫秒） |
| `receipt_status` | `completed`/`failed`/`stopped`/`abandoned` 或 null | 结构化回执里的 `status`，只给结论 |

| 事件字段 | 类型 | 来源（审计记录） |
|---|---|---|
| `at` | RFC3339 UTC | `recorded_at` |
| `project` | 路径 | 事件主体任务所属项目 |
| `task_id` | ULID | 主体 `task_id` |
| `action` | 字符串 | 稳定动作名，如 `board.request`、`task.claim`、`handoff.returned` |
| `phase` | `intent`/`outcome` | 审计阶段 |
| `attempt_id` | `A-` 加 ULID 或 null | 主体 `attempt_id` |

时间格式为 jiff `Timestamp` 的 RFC3339：`Z` 结尾，秒以下按需给出 0–9 位小数；
`window_from` = `window_to` − `window_hours` 小时，按纳秒精确相减，所以跨度恒为整小时。

## 3. 选择、排序与上限

- **items**：全部未结束工作项，加上窗口内有事件的已结束工作项。已结束＝`succeeded`、
  `failed`、`cancelled`、`abandoned`；`approved` 与各 `needs_*` 都算未结束。按
  （`project`, `id`）升序（Rust 字符串字节序，等于 Unicode 码点序），最多 500 条，
  超出保留排在前面的 500 条并置 `items_truncated=true`。
- **events**：窗口内全部项目的任务事件。来源与 `/api/v1/board/timeline` 相同：审计账本里
  主体带 `task_id`、且该任务是黑板工作项的记录；主体不是黑板任务的记录（会话、策略、M0
  任务等）不出现。按（`at`, 追加偏移）升序，最多 1000 条，超出保留最新的 1000 条并置
  `events_truncated=true`。
- 由此成立的两条不变量（前端解码据此核对）：`items_truncated` 为假时，每条事件的任务都在
  `items` 里且项目一致；`events_truncated` 为假时，每个已结束工作项至少有一条事件。

## 4. 审计扫描与完整性

审计账本没有时间或任务索引，只能按偏移翻页。汇总先定位账本尾部，再从尾部整页倒扫：

1. 平面记住上次看到的末条偏移作为提示；每次先用一条记录的探测核实提示仍在账本内
   （账本被换成更短的也会回退到 0），再倍增步长向后探、二分收窄，直到末条落在一页之内。
   稳态下只需两次探测；首次调用约 2·log₂(末条偏移 / 4096) 次。
2. 每页 4096 条倒扫，核对偏移连续（不连续即 503）。某页整页早于「`window_from` 减 10
   分钟」、或扫到账本开头，即确认窗口已覆盖完整。
3. 单次请求最多倒扫 65,536 条。扫满仍未越过窗口起点时，窗口更早部分未读：
   `events_truncated` 与 `items_truncated` 都置真——事件保留已读到的最新部分，
   工作项可能缺少「只在未读部分有事件」的已结束项。

停扫判据依赖一个前提：审计记录在写事务里串行取号并盖时间戳，所以时间随偏移单调不减。
10 分钟余量只兜底小幅时钟回拨；若系统时钟在窗口起点附近回拨超过 10 分钟，窗口边缘的
个别事件可能漏掉。汇总只在读取工作项时持有引擎锁，审计倒扫与实例链查询都不阻塞黑板写入；
因此工作项与事件是相隔很短的两次读取，`generated_at` 标明读取时刻。

## 5. 与 `/api/v1/board` 的关系

- 同一会话门禁、同一许可池、同一 effective 策略哈希；`items` 是全部项目 `BoardView`
  （`board_list_all`）按第 2 节表格投影后的摘要，不按项目参数过滤——人侧所有者本来就能
  逐个项目读 `/api/v1/board`。
- 汇总不给：创建者的实例链、候选暂存与发布收据、交接与回执正文、验收标准、指令、
  输入快照、fence、runner 引用。要这些细节仍读 `/api/v1/board?project=…` 或回放接口。
- 已结束且窗口内没有事件的工作项不出现；需要完整历史用 `/api/v1/board` 与时间线。

## 6. 实例链

`holder_lineage` 与 `/api/v1/board` 的 `instance_lineages` 里以 `holder` 为叶的那条同源：
同样调用实例库的 `instance_lineage(holder)`，把叶→根翻成根→叶，元素 `{id, agent_type}`。
最多四层、同一 agent 类型、叶子是 `holder` 本人。查询失败整份汇总 503，与
`/api/v1/board` 一致。同一 `holder` 在一次汇总里只查一次。

## 7. 安全边界

- 只读元数据：不含任何笔记或产物正文、指令、验收标准、交接与回执正文、审计 payload、
  审计事件 id 与偏移。回执只给四值结论。
- 出现的标识（任务 id、尝试 id、实例 id、agent 类型、项目与路径）都已在 `/api/v1/board`
  对同一所有者可见；`policy_hash` 是摘要，不是凭据。不含会话、CSRF、令牌或任何秘密。
- `title` 是 agent 写的自由文本（创建时整份定义不超过 64 KiB，未限制字符），前端只能按
  文本节点渲染。

## 8. 前端解码与取数

- `parseBoardOverview(value, windowHours?)`：六层字段集合逐字匹配；状态、adapter、尝试阶段、
  回执结论、事件阶段按取值集合核对；标识与路径核长度并拒绝 C0/C1 控制字符，路径另按
  `VaultPath` 语法（NFC、无反斜杠、无空段与 `.`/`..`）；时间必须是真实存在的 RFC3339 UTC；
  窗口为 1–168 整小时且 `generated_at == window_to`；工作项 id 唯一、按码点序排好；
  事件在窗口内且时间不减；并核对第 3 节两条不变量与「持有者、尝试、实例链、进度、回执与
  任务状态」的相互印证（与黑板视图 `boardModel.ts` 同一套规则）。任一不符抛出
  `黑板汇总格式无效，实时状态不可确认。`，不返回部分结果。标题只核非空白与 65,536 长度上限。
- `fetchBoardOverview(windowHours, signal)`：先拒绝 1–168 以外或非整数的小时数（`RangeError`，
  不发请求），再经 `getJson` 走与其它接口相同的会话闸门（认证请求在途 ≤ 2），并要求响应
  窗口跨度等于请求值。`boardOverviewDisabled(error)` 只把 404 认作「黑板未启用」。

## 9. 验证边界

自动测试（2026-09-22，macOS，工具链 1.97.1）：

- `crates/local-server/tests/board_overview.rs`：401/404、400（未知参数、0、169、非数字、
  小数、负数、空值、溢出、重复）、逐层字段集合、`holder_lineage` 与 `/api/v1/board` 同源、
  终态过滤与 1/24/168 小时窗（把已写记录整体挪到三天前的账本装饰器模拟时间流逝）、
  事件升序与归属、真实写入 501 个工作项与 1005 条事件下的两种截断。
- `overview.rs` 单测：尾部定位对任意提示都夹住末条、偏移不连续失败关闭、整页早于停扫线
  即停、时钟余量、扫满上限判不完整、选择排序与截断、只投影元数据。
- `board.rs` 单测：`board_list_all` 覆盖全部项目、与 `board_detail` 同一投影、
  `board_list` 等于其按项目过滤。
- `web/src/test/boardOverviewModel.test.ts`：合法样例、逐层多字段与少字段、取值集合、
  控制字符与路径、时间与窗口、相互印证、码点排序、截断标志与上限、取数与 404 识别。

未覆盖：429 分支没有自动测试（与其它黑板只读接口同一段许可代码）；未在生产规模账本上
量过耗时；Linux 原生回路与 CI 由整合方统一跑；时钟回拨超过 10 分钟的窗口边缘行为只有
文字说明。
