# 供给面与 MCP 外壳契约（M3 首片，2026-09-06 T-033）

> 权威依据：vault [[ADR-013-AI主体双信任模型]]、[[ADR-015-分发外壳与AI身份]]、[[敏感动作枚举表]]「四附」矩阵、[[roadmap]] M3 行。本片只做**接入方（Consumer）**的只读供给：冷启动路牌、白名单供给、类型/实例身份、读取审计、MCP 主外壳。推模式、HTTP 文本兜底、持久派生索引、图 2 属后续片。

> T-037增加HTTP只读文本外壳，四供给操作共用原实现、身份/白名单/审计/新鲜度与预算。
> 短命HTTP会话、路由及发布状态以 [HTTP文本供给](../m3/http-text.md) 为准；首片历史说明保留。
> T-038增加显式客户端资料投影，导出复用HTTP鉴权及审计；本地读取/吊销限制、配置格式与验收见 [客户端投影](../m3/client-projection.md)。
> **2026-09-17 T-069（M4）**：`serve --policy` 接入策略平面后，每次调用先复核三级吊销与该类型能力集，供给视图 ＝ `--allow` ∩ 能力集白名单，
> 无能力集即零权限（握手「应答但一项不给」）；路牌第 3／5／6 项改自 effective；新增 tools `policy_read`／`propose_desired` 与 `GET /read/policy`（T-182 再加只走 MCP 的 `list_proposals`）。
> 本文 §2「只读 ＋ 配置白名单（M4 前固定）」与 §4「四个 tools」是 M3 首片的历史口径；现行以 [策略平面契约](policy-plane.md) 为准。
> **2026-09-26 T-160（ADR-026）**：能力集新增规则名单 `read.rules_whitelist`，规则文件与策略规则母本在接入方的五条读取路径上
> 按同一名单裁决，`policy_read` 的单条 skill 正文改给规范 JSON；见 §10。
> **2026-11-24 T-237（ADR-046）**：同一个端口多一组 `/connect/v1/*`，给只有窄权限的连接器类型，不经本文的会话、路牌和供给视图；
> 这种类型开 MCP、`/read/*` 一律按策略拒绝。见 [连接器契约](connector.md)。

## 1. Module 与装配

| crate | 角色 | 依赖（workspace 内） |
|---|---|---|
| `studiolo-supply` | 纯库：白名单扫描、路牌派生、token 预算、身份、审计事件、读取派生索引 | core、vault-broker（只读）、vault-index、workbench、audit-ledger、rusqlite、tiktoken-rs |
| `studiolo-agent-server` | 二进制 `studiolo-agent`：rmcp Streamable HTTP 壳 + `admin` 子命令 | supply、audit-ledger、axum、rmcp、tokio |

- 只读扫描复用 `vault-broker::ReadOnlyVault`：**每个白名单目录各开一个只读根**，未列入的目录从一开始不进入任何计算；不装配 `FsVaultBroker`，不获得 canonical 写能力。
- 审计写入口唯一：`SqliteAuditLedger::open_existing(<agent 私有目录>/audit)`，与 Auth 账本分库（跨进程不共写一条哈希链）。
- 私有目录 `<data>/`：`audit/audit.sqlite3`（权威账本，由 audit-ledger 打开）与 `agent.sqlite3`（类型摘要、实例登记、读取审计派生索引、outage 记录，由 supply 打开）。目录与文件沿用审计存储同一套规则（绝对路径、0700/0600、逐层拒绝链接、同 owner）。
  **2026-09-17 核实**：T-039 之后另有可选的 `index.sqlite3`（单槽解析缓存，由 `--index-cache` 显式给定的目录持有，见 `crates/supply/src/index_cache/paths.rs:30-36`）。
  架构守卫 `crates/studiolo/tests/architecture_support/source.rs:107-140` 现登记五个固定私有库文件名：`bridge.sqlite3`、`audit.sqlite3`、`auth.sqlite3`、`index.sqlite3`、`agent.sqlite3`；原文「第三个固定文件名」已过时。
- 配置只经命令行参数（本片不读配置文件，避免外壳 crate 获得通用文件读写能力）：`serve --vault <绝对路径> --allow <相对目录>…（可重复）--data <绝对私有目录> --port <端口>`；`--data` 与其下 `audit/` 须由主人预先 `mkdir -m 700` 创建，服务不创建目录。类型 token 明文永不进参数或文件。
  **2026-09-17 核实**：`serve` 现另接受 `--replica-root`（与 `--vault` 互斥，见 §6）、`--replica-max-age-secs` 与 `--index-cache`；解析见 `crates/agent-server/src/options/parser.rs` 的 `"serve"` 分支（2026-09-23 T-154 拆分后位置，按分支名引用不写行号）。

## 2. 身份两级

| 级 | 签发 | 存储 | 权限 |
|---|---|---|---|
| 类型（`claude` / `codex` / `grok`…） | `studiolo-agent admin type-issue --data <绝对私有目录> --type <名>`，32 字节随机 token 只在真实前台控制终端显示一次（门禁见下） | `agent.sqlite3` 的 `agent_type` 表只存 sha256 与签发时间 | 只读 ＋ 配置白名单（M4 前固定） |
| 实例 | 客户端带 `Authorization: Bearer <类型token>` 发 MCP `initialize`；服务端铸 ULID 实例 id，绑定类型与 MCP 会话 id | `agent.sqlite3`：实例 id、类型、会话 id、注册时间、最近活动 | 无独立权限，继承类型 |

**2026-09-17 T-056 文档漂移审计更正**：

- 本仓库从来没有 `identity.sqlite3`；两级身份都存在 §1 的 `agent.sqlite3`
  （`crates/supply/src/database.rs:1`、`crates/supply/src/storage.rs:39-41`，签发见
  `crates/supply/src/database/identity.rs:10-27`）。
- `admin` 子命令一律用 `--data <绝对私有目录>` 定位私有库，**没有** `--config <agent.json>`
  参数；`type-issue` / `type-revoke` 的允许参数恰为 `--data`、`--type`
  （`crates/agent-server/src/options/parser.rs` 的 `"admin"` 分支）。同名类型再次 `type-issue` 会轮换 token 并使旧 token 立即失效。
- **2026-09-17 T-060 已修复并收紧**：T-056 登记的「实现只用 `println!` 写 stdout、
  两个 crate 都没有 TTY 判定」已修。原文「只在 TTY 打印一次」现由实现兑现，且比原文更严。
  门禁在 `crates/agent-server/src/admin/terminal.rs`，做法照搬
  `crates/local-server/src/maintenance/terminal.rs` 与 `studiolo-auth-admin`（见
  [auth-admin.md](auth-admin.md)），未自创第二套：

  - **身份核验**：固定只打开 `/dev/tty`（`RDWR|CLOEXEC|NOCTTY|NONBLOCK`），要求 **stdin、
    stdout、stderr 三者与 `/dev/tty` 同为当前前台控制终端**——`isatty`、
    `tcgetsid == getsid(None)`、`tcgetpgrp == getpgrp`，再比对三个标准流的 `st_rdev` 一致。
    `/dev/tty` 是控制终端别名、设备号不等于实际 PTY，因此它只参与会话／前台组核验，
    不参与设备号比对（这个坑 auth-admin 与维护 CLI 已经踩过）。
  - **交付通道**：明文只写进那个 `/dev/tty` 句柄，带 30 秒绝对期限与 `EAGAIN` 短写重试，
    写入前后各复核一次身份；标准输出只留一行不含秘密的确认。
  - **拒绝语义**：非 TTY、管道、`> file`、改接 stderr、stdin 来自 `/dev/null`、
    以及根本没有控制终端（CI、systemd、nohup、setsid）一律**拒绝且不签发**。
    门禁排在打开私有库之前，因此拒绝路径上不产生任何明文、库中状态不变、
    既有 token 既不轮换也不吊销。**没有降级到 stdout 的分支。**
  - **顺序与回滚**：门禁 → 探写非秘密抬头 → 签发 → 显示明文 → 非秘密确认。
    显示失败立刻 `revoke_type` 回滚，绝不留下一枚主人没拿到的活 token；
    同名再次 `type-issue` 会轮换并解除吊销，所以回滚后无需人工清理。
    回滚本身失败时明确报「可能存在未交付的有效 token」并要求立即 `type-revoke`。
  - **同类出口**：`studiolo_supply::IssuedToken` 不再派生 `Debug`；手写实现只报类型名与
    明文长度，避免 `{:?}`、日志或 panic 消息成为第二个「只显示一次」的出口。
  - **不要求终端的子命令**：`init` / `type-revoke` / `types` / `reads` /
    `rebuild-read-index` / `outages` 只输出非秘密结果，可照常重定向与管道。
  - **比 auth-admin 多要求一条 stdout**：那边秘密是入向（读 OTP），stdout 只承载固定非秘密
    结果所以允许重定向；这里秘密是出向，而 `> token.txt` 恰是主人最自然的手势，
    放行只会让人以为文件里有 token。
  - **待 root 接线**：运维手册 `docs/m3/server-agent.md` 第 86 行
    `… type-issue … | tail -1 > ~/.config/studiolo-agent/claude.token` 在本门禁下会被拒绝，
    须改为在交互终端显示后由主人手工抄进 0600 文件；该文件不在本片写域。

- 无 / 错 / 已吊销 token：HTTP 401（`WWW-Authenticate: Bearer`，无正文），**不进入任何内容计算**，记 `agent.action_denied`（reason=`unauthenticated`，主体匿名，限频见下）。
- 身份库读不出来（SQLite 忙超时、I/O 错、私有存储复核失败）时令牌真假未知：HTTP 503，**不带** `WWW-Authenticate`，正文只给固定类别、不回显底层错误或路径，记 `agent.action_denied`（reason=`identity_unavailable`，认证门这一处主体匿名、限频见下）。不能折成 401：客户端收到 401 会当成令牌失效、转去重新认证（T-180，vault BUG-030）。已建立的会话里任何一处复核读不出来，也只报不可用、不当成吊销：MCP 门禁回 503，tool 返回「身份库不可用」错误（父实例查询同样，不报「父实例无效」），响应流暂停交付，会话保留到下一次再判（POST 响应体的 30 秒期限照旧，见 [MCP 会话边界](../m3/mcp-session-boundaries.md)）。
- **没认出类型的拒绝限频登记**（T-241，2026-11-25 用户定）：上面两条在认证门就拒的，谁都能触发，所以同一入口同一原因每分钟最多登记一条——入口只分 `mcp.request`（`/mcp`）和 `http.request`（`/read/*`），按路由定，`/read/mcp` 也算 `/read/*`，路径和令牌不进名下；原因就是 `unauthenticated`、`identity_unavailable` 两种。两个入口合计最多每分钟 4 条；`/connect/v1/*` 另算自己那一份（[connector.md](connector.md) 第 6 节）。没登记的那些，状态码、答复头、正文和登记的那次一样。账本写不进也算用掉这一分钟；服务重启后重新起算。**所以匿名拒绝的条数不等于请求次数**，一条只说明「那一分钟里至少有一次」。认出类型之后的拒绝（`session_invalid`、`request_capacity`、会话复核时的 `identity_unavailable` 等）照旧逐次登记。
- 同一 MCP 会话内所有 tool 调用共享该实例；实例注册记 `agent_instance.registered`（内容见下一条）；会话结束实例保留记录不删。本片服务端只协商带会话的协议版本（2025-11-25 / 2025-06-18 / 2025-03-26），SEP-2567 无会话模式下的实例身份另行设计。三级吊销属 M4；本片只提供 `admin type-revoke`：类型摘要标记吊销后，**同一会话的下一次调用在门禁处即 401**（记 `unauthenticated`），新握手同样 401；handler 内另有 `type_is_active` 复核作为纵深。
- `agent_instance.registered` 的内容是类型、实例，加下面三格（T-264，2026-11-28；之前的记录没有这三格）。只有这五项，不记正文、令牌、会话号。
  - `parent`：直接的父实例 id（握手带 `studiolo.parent_instance` 的子实例），不是链头；没有父实例是 null，HTTP 文本兜底、连接器从来没有。
    父实例不存在、类型不符或被吊销的登记不成，也就没有这条审计。
  - `negotiated_version`：服务器**实际应答**的协议版本（`InitializeResult.protocolVersion`），不是客户端要的；客户端要的不在上面三个里就是 `2025-11-25`。
    登记写在应答之前，之后握手失败的（如路牌超预算）这一格是本来会应答的版本。
  - `client_reported`：客户端自报的 `clientInfo`，对象 `{"name", "version"}`，两样各去掉控制字符和看不见的字符、最多 64 个字符（截断的末位是 `…`）。
    没人验过，**只当显示，任何授权判断都不读它**；`title` 等其余字段不记。
  - 没有协商的两条路，上面这两格是同一个固定字符串：HTTP 文本兜底 `not_negotiated:http_text`，连接器 `not_negotiated:connector`。
    自报名是对象的才是 MCP，客户端自报什么名字都装不成另外两条路。
  - **只看这一条看不到「只说无会话新版」的客户端**：它不发 `initialize`，在门禁就回 400、没有实例，只留一条 `agent.action_denied`
    （动作 `mcp.request`，reason=`initialize_invalid`）。

## 3. 冷启动路牌（二元验收）

- 交付点：`InitializeResult.instructions` 就是路牌；tool `handshake` 可重取同一内容。
- **可见段**（T-263，vault BUG-061）：Claude Code 只把 `instructions` 的前 4096 个 UTF-16 单元交给模型（汉字 1、emoji 2），
  其余换成「… [truncated]」；`handshake` 的结果不受这个上限管。所以节号不变，排列改成 **1、5、6、3、4、2**，第 2 节的项目清单放最后；
  第 1 节末尾一行写明排法和「末尾出现 [truncated] 就调 handshake 取全文」。可见段＝抬头到第 2 节说明行（含容量提醒），
  派生时按 `CLIENT_VISIBLE_UTF16` 裁：放不下就从当前等级起只把第 5、6、4 节往深裁，不动项目清单；最深一级仍放不下照发、不失败，
  硬约束和续读入口排在最前，仍整节在线内。黑板行在第 4 节标题下一行。
- 六项全部**机器派生**（A1），不读任何人写的「路牌文件」：
  1. 我是谁：类型、实例 id、注册时间；
  2. 白名单内每个项目：`00-总览/INDEX.md` 首表的「当前版本 / 状态 / 最近日志 / 未关闭 BUG」＋「当前重点」首条（截断 200 字）；骨架缺失的字段显示「不可用」；副本用到任一条协议上限的 80% 起，本节第一行是容量提醒（见 §6），机器解析不要把本节首行当成项目数；
  3. 能力摘要：只读；白名单清单；写与审批：无（M3）；
  4. 待我做的事：`04-协调/TASKS.md`「① 进行中」各行（ID / 任务截断 80 字 / 状态）＋「待认领」段落标题；按项目轮流取（第 1 轮每个项目取第 1 条，第 2 轮取第 2 条……），收尾行写明全部项目合计与实际列出的条数；裁剪等级 0～1 时第 2 项的项目行另带该项目的「进行中 n／待认领 m」（T-193）；
  5. 硬约束：白名单内 `通用工作规则.md`（母本）的 `## 准则…` 标题（母本缺失时退回项目 `CLAUDE.md` 快照「准则」节的编号加粗条目）＋ 两处文件里含「一律叠加 / 必叠」的硬绑定句；
  6. 继续拉：tool 清单 ＋ 入口文件（INDEX、TASKS、最新日志、规则母本）的路径与 revision；可读项目不止一个时另有「入口规律」一行，
     各项目的 `CLAUDE.md` 由它说明、不再逐条列（只有一个可读项目时没有这行，`CLAUDE.md` 照旧给入口）。
- 预算：以 `tiktoken-rs` o200k_base 计量 instructions 文本；派生器先按四级裁剪（当前重点字数、进行中行数、待认领条数、入口条数）逼近内部目标 ≤6000（`TARGET_BUDGET_TOKENS`），**裁剪后仍 > 8000 token（`HARD_BUDGET_TOKENS`）即 initialize 失败**（`budget_exceeded`），记 `handshake.budget_exceeded`。**这是二元验收项**，不得降级为告警。真实库首次验收（2026-09-07）：项目/Studiolo + _规则 的路牌 1583 字符、o200k 931 token（cl100k 参考 1105），裁剪等级 0（见 verify-log）。
- 成功记 `handshake.served`（类型、实例、路牌版本＝生成代次、实际 token 数）。

## 4. tools（接入方动作，按「四附」矩阵）

| tool | 输入 | 输出 | 审计 |
|---|---|---|---|
| `handshake` | 无 | 路牌文本 | `handshake.served` |
| `list_entries` | `project`（白名单项） | 路径、revision、字节数、种类（note/attachment） | `content.read`（路径＝项目根） |
| `read_note` | `path`；可选 `lines`／`heading`／`outline` 三选一（见下） | 首行 `revision <hex>`，托管模式再接新鲜度行，然后正文（UTF-8）；上限 4 MiB | `content.read`（路径、revision），只要一段也是同一条 |
| `search` | `project`、`text`、`mode`（full_text/path） | 命中路径、行号、片段（复用 workbench） | `content.read`（每个命中路径各一条，上限 100） |

- `read_note` 只要一段（T-255；只有 MCP 工具，HTTP `/read/note` 仍只给整篇）：
  - 都不给（或 `outline` 为 false）：整篇，和没有这三个参数时逐字节一样。
  - `lines="a-b"`：第 a 到 b 行，从 1 起、两端都含，行号和 `search` 报的是同一套（按 `\n` 切，原来的换行不动）。
    正文前多一行 `# 行 a-b · 共 N 行`。写法不对、从 0 起、反序、越界都回 `invalid_params`；越界那句带总行数。
  - `heading="标题"`：该标题到下一个**同级或更高级**标题之前（含子节），头行同上。可以带 `#`（`## 标题`），带了就连级别一起核；
    同名取第一处，头行写明同名几处。没有这个标题只回固定一句「没有这个标题」，不带别的内容。
  - `outline=true`：头行 `# 大纲 · n 节 · 共 N 行`，之后每个标题一行 `起-止<TAB>## 标题`，井号个数是级别。行段可以直接交给 `lines`，
    标题可以直接交给 `heading`，两种读法逐字节一样。标题超过 10000 个的笔记不给大纲和节（大纲会比原文大好几倍），固定一句叫你用 `lines`。
  - 哪些行算标题和读模型的大纲同一套规则（跳过 frontmatter 和代码围栏，缩进照认，见 `vault-index.md`）；两套不许走岔由契约逐条对照。
  - 先判路径、取到笔记，再看参数（类型也是这时才核，不交给反序列化）：越白名单、不存在、读不了的拒绝、措辞和审计都不因这三个参数改变。
    参数写得对才记 `content.read`（整篇的 revision），记了再切，所以「行段越界」「没有这个标题」也各有一条读取记录；参数写得不对不记。
- 审计**先于**返回：`content.read` 追加失败即拒绝返回，错误 `audit_unavailable`。
- 白名单外路径、隐藏路径、越界：拒绝并记 `agent.action_denied`（reason=`outside_allowlist` / `unsafe_path`）；不回显宿主路径。
  接入策略平面后，前缀内但不在规则名单的规则文件记 `rule_not_whitelisted`，对外与白名单外同一句（§10）。
- 不提供写、不提供整库视图、不提供「先给个只读」默认值（未列入即拒绝）。

## 5. 读取审计查询

- 权威：账本 `content.read` / `handshake.*` / `agent.action_denied` 事件，payload 只含类型、实例、路径、revision、tool、token 数，不含正文与 token 秘密。
- 派生索引（`agent.sqlite3` 的 `reads` 表）：`(recorded_at_ms, recorded_at, type, instance, path, revision, tool, ledger_offset)`，账本追加成功后写入，同一账本偏移只记一次；`admin reads --type --instance --since --until --path-prefix --limit` 由人查询（时间按 Unix 毫秒比较）；`admin rebuild-read-index` 从账本 `content.read` 事件重放重建。索引损坏不影响供给，只影响查询。

## 6. 新鲜度与降级

- 直接 `--vault` 模式沿用首片契约：启动全扫白名单子树建代次；后台每 60 秒重扫；`read_note` 直接读当前文件并返回其 revision。扫描诊断降级为路牌里的「诊断 n 条」。此模式没有远端同步回执，不声称资料已从来源同步。
- T-036 增加显式 `--replica-root` 托管模式，与 `--vault` 互斥。每次内容请求在专用校验线程上串行验证完整批次与 SHA-256 清单（T-185 起；此前在 tokio 阻塞线程池上，进程 `mlockall` 后各线程 malloc arena 释放的页退不回去，并发请求把常驻集叠成几份校验峰值，2026-10-15 生产被单元 `MemoryMax=512M` 杀掉，见 vault BUG-034 与 §9「内存预算」），四工具使用该次完整内存快照；附件只供给元信息（路径、大小），**每次请求只核同一次安全扫描得到的类型与字节长度，不整读、不算哈希**；附件字节的完整性由接收端在副本到达时逐文件核 SHA-256 保证（`scripts/replica_store.py`）。笔记正文会对外供给，仍每次整读并核 SHA-256，同大小字节漂移立即阻断；正文用的是同一次安全扫描读到的字节（T-215 起，vault BUG-039：此前每篇再按路径单独重读一遍，每读一篇都把所在目录的名字整个枚举两遍查大小写撞名，耗时随同目录篇数平方增长，4000 篇挤在一个目录时单次校验 23.8 秒；扫描本身已对每层目录枚举一次、查过撞名，并按不跟随链接、不跨设备、私有权限、读前读后元数据不变的规矩读每一篇，两轮捕获仍逐字节比对）。（2026-09-22 T-108 改，见 vault ADR-023：此前附件也每次核哈希，纳入 199 MB 附件后每请求要多约 1.5 秒。）控制文件无效、内容漂移、任意扫描诊断都拒绝内容；不静默退回直接模式。
- 托管模式每个结果包含 generation、source_digest、source_observed_at_ms、received_at_ms、age_secs、状态。默认超过 300 秒为 stale；最近同步失败为 sync_failed，同时保留 stale 布尔值。旧完整副本可在明确标记后只读使用，不能据此开展实时协作。只重扫服务器目录不能刷新来源确认时间。新鲜度是路牌强制字段，同样受 ≤8000 token 硬门槛约束。过期或同步失败时，机器行的下一行有一句给人看的提示（资料停在哪一刻 UTC、停了多久、涉及进度的结论先向用户核实）；新鲜时没有，机器行本身一个字符都不变。副本用到任一条协议上限（Markdown 合计、文件总量、可见条目数、清单字节）的 80% 起，路牌（只在路牌，不在逐次的内容响应里）第 2 节开头加一句容量提醒：哪几条到线、用了多少、上限多少、越线后同步会整批失败、请提醒用户趁早调上限（T-215，vault BUG-039：上次越线一个月后才被发现）。它不接在新鲜度后面：部署验收 `scripts/check-agent-http.py` 与客户端投影 `scripts/client_projection_fetch.py` 按固定行号找身份段（第 3 或第 4 行），多一行就判路牌无效（T-215 复查）。用量是全副本的数，只给读得到副本里每个条目的接入方（与读取路径同一个 `grant.allows` 判定，含规则名单）：只读得到一部分的，减去自己读得到的就能推出读不到的项目有多大、有多少附件，所以读不全就不给。条目数只数可见项，Mac 端捕获时连隐藏项一起数，所以这一条按偏少的数提醒。
- 完整协议、边界与启用状态见 [资料同步与新鲜度](../m3/replica-freshness.md)，操作步骤见 [自动同步运维](../m3/replica-operations.md)。本片是服务器副本同步；ADR-015客户端独立投影由T-038实现，实际客户端安装仍另验。
- 审计不可用（账本追加失败，含存储复核失败）：initialize 仍应答，但 instructions 只含「审计不可用 ＋ outage id ＋ 本次读写全部拒绝」，不含任何 vault 内容；所有 tool 返回 `audit_unavailable`；outage 起始、拒绝计数与结束时间落 `agent.sqlite3` 的 `outage` 表（同一 outage 只记一行，计数有界），账本恢复后第一次成功追加即结束窗口。带外告警与健康页红属运维片，本片只留持久痕迹（`admin outages`）。

## 7. 错误模式

| 错误 | 触发 | 对外 |
|---|---|---|
| `unauthenticated` | 无/错/吊销 token | HTTP 401，无正文细节 |
| `identity_unavailable` | 身份库读不出来（认证、会话复核或父实例查询时） | HTTP 503，不带 `WWW-Authenticate`，固定正文；会话保留，下一次请求再判；tool 内返回「身份库不可用」错误（见 [MCP 会话边界](../m3/mcp-session-boundaries.md)） |
| `session_invalid` | MCP 会话号查不到、已过期、已关闭，或属于别的类型或凭据 | HTTP 404，不带 `WWW-Authenticate`；客户端不带会话号重新 initialize（见 [MCP 会话边界](../m3/mcp-session-boundaries.md)） |
| `session_required` | GET／DELETE 不带会话号 | HTTP 400 |
| `server_stopping` | 收到 SIGTERM／SIGINT 之后到达的新会话、新请求、新 GET 流（MCP 与 `/read/*` 同样） | HTTP 503；MCP 的 DELETE 照常；已进门的请求照常做完（§9） |
| `budget_exceeded` | 路牌超 8000 token | initialize 失败，JSON-RPC 错误含实际 token 数 |
| `outside_allowlist` / `unsafe_path` | 越界读取 | tool 错误，已审计 |
| `rule_not_whitelisted` | 读能力集规则名单外的规则文件或规则母本（§10） | 与白名单外（`read_note`）或不存在（`policy_read`）同一对外错误，已审计 |
| `audit_unavailable` | 账本不可用 | 降级握手 / tool 拒绝 |
| `not_found` | 白名单内路径不存在 | tool 错误，已审计为拒绝 |
| `read_note` 的参数不对 | `lines` 写法不对／越界、`heading` 是空的、类型不对、三样同时给了两样以上、标题太多给不出大纲 | JSON-RPC `-32602`，固定措辞 |
| `read_note` 没有这个标题 | `heading` 在这篇里找不到 | JSON-RPC `-32002`「没有这个标题」，读取已审计 |

## 8. 契约测试清单

- 对象安全与依赖方向：supply 不依赖 axum/rmcp；agent-server 不依赖 FsVaultBroker、ControlDb、auth。
- 预算二元：构造超预算 INDEX 夹具 → initialize 失败且账本有 `handshake.budget_exceeded`；正常夹具 → instructions ≤8000 且六项齐全。
- 可见段（`crates/agent-server/tests/signpost_visible_contract.rs`，白名单照生产单元模板的 `--allow`、工具说明用真实 `TOOLS`）：
  生产规模下线内有除项目清单以外的全部内容且不触发裁剪；长待办、半数 emoji 的待办、四条上限都到线的容量提醒下仍在线内，且项目清单不受影响；
  白名单涨到能力段自己放不下时硬约束与续读入口仍整节在线内。4096 在测试里写死，不引产品常量。
- 身份：无 token 401；错 token 401 且 `agent.action_denied`（匿名的按入口和原因限频，认出类型的逐条，`mcp_cases/unrecognized.rs`、`auth_private/unrecognized.rs`）；正确 token → 实例落库并绑定会话；吊销类型后立即 401。身份库读不出来（测试把私有库文件改成 0640）→ 503 不带 challenge、会话不丢，恢复后同一令牌和会话照常可用（`mcp_transport_cases/identity_store.rs`、`http_text/availability.rs`、`mcp_transport_private/identity_outage.rs`）。
- 类型 token 显示门禁（真实 PTY 驱动实际二进制，`crates/agent-server/tests/admin_terminal_contract.rs`
  ＋ `tests/support/admin_terminal.py`）：真实控制终端上恰好显示一次；无控制终端 / 全管道 /
  `> file` / 管道接 stdout / 改接 stderr / stdin 来自 `/dev/null` 六种形状各自被拒绝，
  **并实际读回被改接的那个文件或管道断言其中没有 token**，且一枚都没签发；
  被拒绝的一次调用不改变既有类型的签发时间与有效状态，终端恢复后照常轮换出新 token，
  不复显上一枚；`IssuedToken` 的 `Debug` 不复述明文。
- 秘密驻留面（真实二进制，`crates/agent-server/tests/process_residency.rs`）：`--help` 与一条
  注定失败的管理命令各跑一次实际 `studiolo-agent`，逐项分类**恰好记录一次**且排在参数解析之前，
  证明收紧挂在入口而不是挂在某条命令上；Linux 上另起一个真实 `serve`，由父进程从
  `/proc/<pid>/` 回读锁定页数、核心转储限额与环境块可读性。契约见
  [秘密驻留面](../m2/secret-residency.md)。
- 停服（真实二进制，`crates/agent-server/tests/shutdown_contract.rs`，T-184／vault BUG-033）：没有会话、
  开着 GET SSE 流、工具调用卡在半路（假黑板扣住）三种情形各发一次 SIGTERM，进程 5 秒内以退出码 0 退出、
  流正常结束，卡在半路的那次结果完整送到客户端，期间新连接拿不到新会话。关停后两个外壳回 503、
  排空后关全部会话、做不完的请求只能拖到 10 秒上限：`mcp_transport_private/shutdown.rs`、
  `http_text_private/stopping.rs`。
- 常驻内存（真实 loopback 服务，`crates/agent-server/tests/replica_memory_contract.rs`，T-185／vault BUG-034）：
  一次性子进程先与生产一样 `mlockall`，托管副本放 192 篇虚构笔记（约 2.3 MiB），预热后打 2 轮 16 路并发
  `/read/note`，常驻匿名内存（`RssAnon`）的增长不得超过三份正文加 8 MiB。校验还在阻塞线程池上时同一测试涨
  约 142 MiB，改到专用校验线程后涨约 1.5 MiB。专用线程本身的语义与源码守卫在 `verifier_private/queue.rs`：
  16 路并发都落在同一条线程上、调用方已放弃的排队任务不再做、一次 panic 只让投它的调用方失败；
  `src/` 里不许再出现 `spawn_blocking`、`block_in_place`、另起线程，重扫只在请求供给与后台定时两处投递。
- 满额容量（真实 loopback 服务，`crates/agent-server/tests/replica_capacity_contract.rs`，T-215／vault BUG-039；
  只在 release 构建里跑，ci 套 `replica-capacity` 一步）：按 `NOTES_TOTAL_LIMIT` 生成满额副本（真实库的篇幅分布、
  标题与 wikilink 密度，正文虚构，全挤在一个目录），一次性子进程先 `mlockall`、开索引缓存，打 2 轮 16 路并发
  `/read/note`，再连换 3 个各改几十篇的新代次、每次再打 2 轮。最慢一个请求不超过 10 秒、常驻匿名内存不超过 384 MiB
  （单元 `MemoryMax=512M` 的四分之三；每代打完时的 `RssAnon` 和峰值上界都算：峰值按 `VmHWM` 减加固后、起服务前读的 `RssFile` 估，`MCL_ONFAULT` 下 `RssFile` 只增不减），每一代的派生
  索引缓存都写得进去（`persistence: Ready`），路牌第 2 节开头带容量提醒。上限往上调、或者整读校验又变成随篇数超线性，
  它先红：旧的逐篇重读在 24 MiB 时最慢 16.2 秒（红），修后 1.0 秒。两个运行包（`scripts/package-agent.sh`、
  `scripts/package-preview.sh`）都不带它：两处的 `PACKAGE_EXCLUDED_TESTS` 逐项相等，名单里的目标必须存在、在 ci 套里有专门一步，
  凡是 debug 下不跑、只在 release 跑的测试目标都必须在名单里（`test-agent-package.py` 守着：认 `cfg_attr(debug_assertions, ignore…)`
  与 `cfg(not(debug_assertions))` 两种写法，这类用例不许写在测试目标的子模块里，测试里不许用运行时 `cfg!(debug_assertions)`）。部署验收在生产主机上原生跑
  运行包里的测试，满额负载不该和线上服务抢内存。
- 白名单：读 `School/...` 或 `../` → 拒绝＋审计；`list_entries` 只列白名单项目。
- 登记审计（T-264）：MCP 按客户端要的七种版本逐个核对审计与应答一致、整条相等；自报名超长、恰好 64、为空、带控制字符和看不见的字符；
  换自报名不改任何读与拒的结果（`crates/agent-server/tests/mcp_cases/registered.rs`）。HTTP 文本兜底、连接器各整条相等
  （`http_text/contracts.rs`、`policy_cases/connector.rs`）。父实例：根、子、孙三代整条相等，登记不成的没有审计（`policy_contract.rs`）。
- `read_note` 只要一段（`crates/agent-server/tests/read_note_slice_contract.rs`，T-255）：不给新参数逐字节不变；行段、越界、
  反序、0 行；标题节的边界、同名、带 `#`；没有标题只回一句；大纲与读模型的大纲逐条一致、区间与 `lines`／`heading` 逐字节一致；
  五种结果的 `content.read` 与整篇读完全相同；越白名单的拒绝措辞与审计不变；三选一；托管模式新鲜度行仍在前。
- 审计先于返回：注入失败账本 → `read_note` 拒绝且无正文返回；降级握手不含 vault 内容。
- 四维查询：两个类型两个实例各读若干路径 → `admin reads` 按类型/实例/时间/路径前缀过滤正确；重建索引与增量一致。
- 真实 loopback：用 rmcp 客户端跑完整 initialize → handshake → list → read → search。
- 真实验收（人）：`claude mcp add --transport http http://127.0.0.1:4400/mcp --header "Authorization: Bearer …"`，Claude Code 冷启动读到 Studiolo 路牌并按需读一篇；`admin reads` 查到该次读取。

## 9. 部署形态（T-034，2026-09-07）

- `serve` 只监听 `127.0.0.1:<port>`，不读 Host/Origin：公网入口由专用 Cloudflare Tunnel 的独立主机名转发到回环端口并把 Host 覆写为 `127.0.0.1:<port>`；应用不信任任何转发头，身份只来自 bearer 类型 token。
- 优雅关停：SIGINT 与 SIGTERM 等价（systemd 默认 `KillSignal=SIGTERM`）；关停期间不再接受新会话，已开始的供给完成后退出，全程有上限（T-184，vault BUG-033）：
  - 信号处理器在绑定端口之后、宣布监听之前装好：监听一宣布，SIGTERM 就走下面的关停，不会被默认动作直接杀掉；更早的启动阶段（首扫、建索引）还没有连接要收尾，SIGTERM 按默认动作立即退出；
  - 收到信号后关闭监听、关掉空闲连接；两个外壳对新会话、新请求、新 GET 流回 503（`server_stopping`），MCP 的 DELETE 照常；
  - 已进门的 MCP 请求做完后关闭全部会话，客户端开着的 GET SSE 流随之正常结束；最多等 10 秒，到点强制关会话（响应截断）并在标准错误记一行；
  - 后台重扫不再开新的一轮，正在跑的那次做完；
  - 从收到信号起 15 秒仍有连接没收尾，`serve` 以退出码 1 退出；之后运行时再给阻塞线程 2 秒（重扫与副本校验不在其列：它们在专用校验线程上，后台那次由 `serve` 等它做完，没人等结果的那次随进程退出结束，见下文「内存预算」）。合计 17 秒，小于 agent 各单元的 `TimeoutStopSec=20`，改任何一边都要保持这个关系。
- T-034 已部署服务器使用 `--vault` 读取原只读副本。T-036 新发布模板使用 `--replica-root`；先完成首批完整同步，再按运维步骤更换 MCP 单元。两种形态白名单均为 `项目/Studiolo`、`_规则`，身份、审计与预算要求相同。
- 未加 Cloudflare Access：服务 token 秘密不能经 agent 之手；主人在面板签发后可再挂 Access 应用（policy 为 Service Auth），届时客户端多带两条 `CF-Access-Client-*` 头即可，本契约不变。
- **部署前置（T-065，2026-09-17）**：`studiolo-agent` 自本次起与另外三个持明文秘密的程序
  共用秘密驻留面加固（`studiolo-process-safety`），Linux 上 `mlockall` 锁住持有类型 token 与
  bearer 的页，`RLIMIT_MEMLOCK` 不足时**拒绝启动**。两个单元模板都已加 `LimitMEMLOCK=infinity`，
  升级时必须连同新单元一起安装，只换二进制会起不来。见
  [秘密驻留面](../m2/secret-residency.md) §5 与 `docs/m3/replica-operations.md` §2。
- **内存预算（T-185，vault BUG-034）**：单元 `MemoryMax=512M`。进程 `mlockall` 之后，glibc 释放锁定页用的
  `madvise(MADV_DONTNEED)` 一律失败，释放掉的堆页留在分配它的那条线程的 malloc arena 里，常驻集只涨不退
  （[秘密驻留面](../m2/secret-residency.md) §5.1）。所以副本校验、后台重扫与首扫只在一条专用线程上做
  （`crates/agent-server/src/verifier.rs`），常驻上界是「同时在用的几份代次 + 一次校验」：请求拿到的代次用完即放，
  校验一个个出队，不再随做过校验的线程数相乘，也不随运行时长累加；排队时调用方已放弃的那次不再做。
  并发到达的请求共用**受理之后才开始、并且成功**的最近一次校验（T-211，`crates/agent-server/src/shared_scan.rs`）：
  每个请求拿到的仍是它到达之后重新完整校验过的代次，但 N 路同时到达最多做两次整读，不再是 N 次
  （生产 14 MiB、16 路并发，最后一个从约 40 秒降到两次校验的时间）。最近一次校验失败了就各自重做，
  失败不外传，也不退回去用失败之前的旧结果；一个接一个到达的请求仍然一人一次。
  以后凡是整读副本或做大块临时分配的新路径，都要放到这条线程上，不能用 `spawn_blocking`，也不能直接在
  tokio 工作线程上做；§8「常驻内存」两条测试守着这一点。
  正文只存一份（T-215）：快照的 `Note.content`、建模输入 `MarkdownDocument` 与读模型的 `DocumentView.content`
  都是同一个 `Arc<str>`，broker 读文件时按长度一次分配到位。原先同一份正文在换代次时同时存三四份（读缓冲倍增
  余量、快照、整份克隆的文档表、读模型里的一份），108 计数分配器实测换代峰值从正文的 8.4 倍降到 4.6 倍。
  正文没变时（绝大多数请求、每 60 秒的重扫）读模型复用上一代的，新代次的快照也改用上一代那份正文（逐字节比过才换，
  `AppState::replace_generation`）；不换的话快照和读模型各持一份（T-215 复查发现，满额常驻从 301 MiB 降到 284 MiB；
  4.6 倍是修这一处之前测的，没重测）。
  Markdown 合计上限（`NOTES_TOTAL_LIMIT`）能定多大，由 §8「满额容量」那条契约在 384 MiB 内能否通过来定。
- 服务器安装、目录约定与实际部署记录见 `docs/m3/server-agent.md`。

## 10. 规则级读权限（T-160，2026-09-26，vault ADR-026）

- **规则文件**＝直接位于 `_规则/` 下的 `.md` 文件，文件名即规则 subject，与是否已发布到策略平面无关（D9）；
  `_规则/` 下的子目录（`references/`、`归档/`、`skills/` 等）不是规则文件，照项目白名单；`项目/*/CLAUDE.md` 是项目文件（D11）。
  规则根名与判定只在 `crates/supply/src/rules_files.rs` 定义一处（`RULES_ROOT`、`rule_subject`）。
- **单一裁决点**（D10）：`Grant::allows` ＝ 供给视图前缀判定 ∧ `allows_rule_file`；规则文件须在能力集
  `read.rules_whitelist` 内，非规则文件一律放行；策略规则母本走 `allows_rule`，同一份名单。
  未接策略平面的 M3 兼容模式不做规则过滤。能力集缺该字段即零规则（D4）。人侧永远全读（D5／D12）。
- **五条读取路径同一判定**（D5）：

| 路径 | 名单外的规则 | 审计 |
|---|---|---|
| `read_note` | 先判前缀：前缀外仍是 `outside_allowlist`；前缀内的名单外规则文件拒绝，对外同一句「路径不在供给白名单内」（HTTP 400） | `agent.action_denied{reason=rule_not_whitelisted}` |
| `list_entries` | 不进清单，计数与行数一致 | 不单独记拒绝 |
| `search` | 项目范围和名单都在计数之前生效：名单外的规则文件不命中，也不占命中名额；100 条上限和「已截断」只数所请求项目内、名单内的命中，别的项目命中再多也不挤占（T-192，BUG-036） | 不单独记拒绝；被滤掉的路径不产生 `content.read` |
| `policy_read kind=rules` | 清单里不出现（计数一致）；单条对外与不存在同一句「规则母本不存在」（HTTP 404） | 单条记 `rule_not_whitelisted`；名单内而不存在记 `not_found` |
| 握手路牌 | 第 5 项准则与硬绑定句、第 6 项入口都不取名单外的策略规则与 vault 规则文件（库文件兜底同一名单） | — |

- `policy_read` 规则读取所见的 effective 哈希须等于本次授权复核时的哈希（名单与条目同属一版），不等即失败关闭
  「策略在本次调用途中发布了新版本，本次拒绝供给，请重试」。
- **`policy_read` 输出**：首行 `# 策略版本 <12 位>`。规则清单次行 `N 份规则母本`，其后逐行 `subject\tv<n>\t<sha256 前 12 位>`；
  规则单条 `policy://rules/<subject>@v<n> sha256 <64 位>` 行后接正文原字节。skills 清单次行 `N 个 skill（…）`，其后逐行
  `名\tv<n>\t摘要\t触发（以 " | " 相连）`，摘要与触发里的控制字符与 U+2028／U+2029 压成空格，一个 skill 恰占一行；
  skill 单条 `policy://skills/<名>@v<n> sha256 <64 位>` 行后接**入库的规范 JSON 原文**
  （`{"name","summary","triggers","markdown"}`，serde 字段序、紧凑分隔、非 ASCII 原样），sha256 即这段字节的摘要，
  读方按收到的字节核对，不再从「摘要：…／触发：…」文本还原（待拍板清单第 9 条，用户 2026-09-23 选 (a)）。
- 契约测试：`crates/agent-server/tests/rules_access_contract.rs`（五条路径、对外同一错误与审计原因、子目录、缺字段零规则、
  M3 兼容模式不过滤、HTTP 外壳同一判定）、`crates/supply/tests/rules_files_contract.rs`（判定与路牌库文件兜底）、
  `crates/policy/tests/rules_whitelist_contract.rs`（能力集字段校验与扩大／收窄项）。


## 新根恢复（T-212，2026-11-16）

两个入口，都只打开封存里放出来的已有 `agent.sqlite3`（不建、不迁移）：

- `verify_restored_identities(目录, 清单水位)`：不改库的内容，按封存时同一套规则重算水位，逐项相等才算过。编排
  第一次做这一步之前调它，核过了才在恢复记录里记「开始」（Codex 第二意见第 3 条：以前核对和吊销是同一个调用，
  「开始」记在核对之前，核不过的库在接手时会被当成核过的直接吊销）。它走身份库唯一的打开入口，会把封存导出时
  的 DELETE 模式切回平时运行的 WAL；这一项不进水位。
- `invalidate_restored_identities(目录, now)`：吊销全部类型（旧类型 token 一律失效），核「没有一个类型还有效」，
  交回全部实例标识，由编排在策略库逐个记实例吊销。同一份库再做一遍什么都不变，接手时只做这一步。

之后由主人在现有界面重新签发类型 token、AI 重新登记。身份库在新根上先放在 `agent.held` 下，吊销做完才改名成
`agent`：之前 agent-server 按正式路径打不开它。接入方账本（`<agent 私有目录>/audit`）在恢复里只核链尾、不写。
见[新根恢复](new-root-restore.md)。

## 人侧身份视图与管理输出（T202）

人侧列表经 `AgentIdentityView` 只读观察身份库，不取得 token 或会话摘要。`session_digest` 绑定的是服务端随机 `session_ref` 的 SHA-256，非客户端提供的会话标识。`studiolo-agent admin instances` 保留原四列顺序并在末尾增加父实例号（无父为 `—`）；读取改用只读视图。网页契约见 [接入方列表](agents-list.md)。
