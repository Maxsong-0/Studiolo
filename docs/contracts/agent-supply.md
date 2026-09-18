# 供给面与 MCP 外壳契约（M3 首片，2026-09-06 T-033）

> 权威依据：vault [[ADR-013-AI主体双信任模型]]、[[ADR-015-分发外壳与AI身份]]、[[敏感动作枚举表]]「四附」矩阵、[[roadmap]] M3 行。本片只做**接入方（Consumer）**的只读供给：冷启动路牌、白名单供给、类型/实例身份、读取审计、MCP 主外壳。推模式、HTTP 文本兜底、持久派生索引、图 2 属后续片。

> T-037增加HTTP只读文本外壳，四供给操作共用原实现、身份/白名单/审计/新鲜度与预算。
> 短命HTTP会话、路由及发布状态以 [HTTP文本供给](../m3/http-text.md) 为准；首片历史说明保留。
> T-038增加显式客户端资料投影，导出复用HTTP鉴权及审计；本地读取/吊销限制、配置格式与验收见 [客户端投影](../m3/client-projection.md)。

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
  **2026-09-17 核实**：`serve` 现另接受 `--replica-root`（与 `--vault` 互斥，见 §6）、`--replica-max-age-secs` 与 `--index-cache`；解析见 `crates/agent-server/src/options.rs:138-200`。

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
  （`crates/agent-server/src/options.rs:202-256`）。同名类型再次 `type-issue` 会轮换 token 并使旧 token 立即失效。
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

- 无 / 错 / 已吊销 token：HTTP 401（`WWW-Authenticate: Bearer`，无正文），**不进入任何内容计算**，记 `agent.action_denied`（reason=`unauthenticated`，主体匿名）。
- 同一 MCP 会话内所有 tool 调用共享该实例；实例注册记 `agent_instance.registered`；会话结束实例保留记录不删。本片服务端只协商带会话的协议版本（2025-11-25 / 2025-06-18 / 2025-03-26），SEP-2567 无会话模式下的实例身份另行设计。三级吊销属 M4；本片只提供 `admin type-revoke`：类型摘要标记吊销后，**同一会话的下一次调用在门禁处即 401**（记 `unauthenticated`），新握手同样 401；handler 内另有 `type_is_active` 复核作为纵深。

## 3. 冷启动路牌（二元验收）

- 交付点：`InitializeResult.instructions` 就是路牌；tool `handshake` 可重取同一内容。
- 六项全部**机器派生**（A1），不读任何人写的「路牌文件」：
  1. 我是谁：类型、实例 id、注册时间；
  2. 白名单内每个项目：`00-总览/INDEX.md` 首表的「当前版本 / 状态 / 最近日志 / 未关闭 BUG」＋「当前重点」首条（截断 200 字）；骨架缺失的字段显示「不可用」；
  3. 能力摘要：只读；白名单清单；写与审批：无（M3）；
  4. 待我做的事：`04-协调/TASKS.md`「① 进行中」各行（ID / 任务截断 80 字 / 状态）＋「待认领」段落标题；
  5. 硬约束：白名单内 `通用工作规则.md`（母本）的 `## 准则…` 标题（母本缺失时退回项目 `CLAUDE.md` 快照「准则」节的编号加粗条目）＋ 两处文件里含「一律叠加 / 必叠」的硬绑定句；
  6. 继续拉：tool 清单 ＋ 入口文件（INDEX、TASKS、最新日志、规则母本）的路径与 revision。
- 预算：以 `tiktoken-rs` o200k_base 计量 instructions 文本；派生器先按四级裁剪（当前重点字数、进行中行数、待认领条数、入口条数）逼近内部目标 ≤1500，**裁剪后仍 > 2000 token 即 initialize 失败**（`budget_exceeded`），记 `handshake.budget_exceeded`。**这是二元验收项**，不得降级为告警。真实库首次验收（2026-09-07）：项目/Studiolo + _规则 的路牌 1583 字符、o200k 931 token（cl100k 参考 1105），裁剪等级 0（见 verify-log）。
- 成功记 `handshake.served`（类型、实例、路牌版本＝生成代次、实际 token 数）。

## 4. tools（接入方动作，按「四附」矩阵）

| tool | 输入 | 输出 | 审计 |
|---|---|---|---|
| `handshake` | 无 | 路牌文本 | `handshake.served` |
| `list_entries` | `project`（白名单项） | 路径、revision、字节数、种类（note/attachment） | `content.read`（路径＝项目根） |
| `read_note` | `path` | 正文（UTF-8）、revision、字节数；上限 4 MiB | `content.read`（路径、revision）|
| `search` | `project`、`text`、`mode`（full_text/path） | 命中路径、行号、片段（复用 workbench） | `content.read`（每个命中路径各一条，上限 100） |

- 审计**先于**返回：`content.read` 追加失败即拒绝返回，错误 `audit_unavailable`。
- 白名单外路径、隐藏路径、越界：拒绝并记 `agent.action_denied`（reason=`outside_allowlist` / `unsafe_path`）；不回显宿主路径。
- 不提供写、不提供整库视图、不提供「先给个只读」默认值（未列入即拒绝）。

## 5. 读取审计查询

- 权威：账本 `content.read` / `handshake.*` / `agent.action_denied` 事件，payload 只含类型、实例、路径、revision、tool、token 数，不含正文与 token 秘密。
- 派生索引（`agent.sqlite3` 的 `reads` 表）：`(recorded_at_ms, recorded_at, type, instance, path, revision, tool, ledger_offset)`，账本追加成功后写入，同一账本偏移只记一次；`admin reads --type --instance --since --until --path-prefix --limit` 由人查询（时间按 Unix 毫秒比较）；`admin rebuild-read-index` 从账本 `content.read` 事件重放重建。索引损坏不影响供给，只影响查询。

## 6. 新鲜度与降级

- 直接 `--vault` 模式沿用首片契约：启动全扫白名单子树建代次；后台每 60 秒重扫；`read_note` 直接读当前文件并返回其 revision。扫描诊断降级为路牌里的「诊断 n 条」。此模式没有远端同步回执，不声称资料已从来源同步。
- T-036 增加显式 `--replica-root` 托管模式，与 `--vault` 互斥。每次内容请求在阻塞线程串行验证完整批次与 SHA-256 清单，四工具使用该次完整内存快照；附件同样验证字节但只供给元信息。控制文件无效、内容漂移、任意扫描诊断都拒绝内容；不静默退回直接模式。
- 托管模式每个结果包含 generation、source_digest、source_observed_at_ms、received_at_ms、age_secs、状态。默认超过 300 秒为 stale；最近同步失败为 sync_failed，同时保留 stale 布尔值。旧完整副本可在明确标记后只读使用，不能据此开展实时协作。只重扫服务器目录不能刷新来源确认时间。新鲜度是路牌强制字段，同样受 ≤2000 token 硬门槛约束。
- 完整协议、边界与启用状态见 [资料同步与新鲜度](../m3/replica-freshness.md)，操作步骤见 [自动同步运维](../m3/replica-operations.md)。本片是服务器副本同步；ADR-015客户端独立投影由T-038实现，实际客户端安装仍另验。
- 审计不可用（账本追加失败，含存储复核失败）：initialize 仍应答，但 instructions 只含「审计不可用 ＋ outage id ＋ 本次读写全部拒绝」，不含任何 vault 内容；所有 tool 返回 `audit_unavailable`；outage 起始、拒绝计数与结束时间落 `agent.sqlite3` 的 `outage` 表（同一 outage 只记一行，计数有界），账本恢复后第一次成功追加即结束窗口。带外告警与健康页红属运维片，本片只留持久痕迹（`admin outages`）。

## 7. 错误模式

| 错误 | 触发 | 对外 |
|---|---|---|
| `unauthenticated` | 无/错/吊销 token | HTTP 401，无正文细节 |
| `budget_exceeded` | 路牌超 2000 token | initialize 失败，JSON-RPC 错误含实际 token 数 |
| `outside_allowlist` / `unsafe_path` | 越界读取 | tool 错误，已审计 |
| `audit_unavailable` | 账本不可用 | 降级握手 / tool 拒绝 |
| `not_found` | 白名单内路径不存在 | tool 错误，已审计为拒绝 |

## 8. 契约测试清单

- 对象安全与依赖方向：supply 不依赖 axum/rmcp；agent-server 不依赖 FsVaultBroker、ControlDb、auth。
- 预算二元：构造超预算 INDEX 夹具 → initialize 失败且账本有 `handshake.budget_exceeded`；正常夹具 → instructions ≤2000 且六项齐全。
- 身份：无 token 401；错 token 401 且 `agent.action_denied`；正确 token → 实例落库并绑定会话；吊销类型后立即 401。
- 类型 token 显示门禁（真实 PTY 驱动实际二进制，`crates/agent-server/tests/admin_terminal_contract.rs`
  ＋ `tests/support/admin_terminal.py`）：真实控制终端上恰好显示一次；无控制终端 / 全管道 /
  `> file` / 管道接 stdout / 改接 stderr / stdin 来自 `/dev/null` 六种形状各自被拒绝，
  **并实际读回被改接的那个文件或管道断言其中没有 token**，且一枚都没签发；
  被拒绝的一次调用不改变既有类型的签发时间与有效状态，终端恢复后照常轮换出新 token，
  不复显上一枚；`IssuedToken` 的 `Debug` 不复述明文。
- 白名单：读 `School/...` 或 `../` → 拒绝＋审计；`list_entries` 只列白名单项目。
- 审计先于返回：注入失败账本 → `read_note` 拒绝且无正文返回；降级握手不含 vault 内容。
- 四维查询：两个类型两个实例各读若干路径 → `admin reads` 按类型/实例/时间/路径前缀过滤正确；重建索引与增量一致。
- 真实 loopback：用 rmcp 客户端跑完整 initialize → handshake → list → read → search。
- 真实验收（人）：`claude mcp add --transport http http://127.0.0.1:4400/mcp --header "Authorization: Bearer …"`，Claude Code 冷启动读到 Studiolo 路牌并按需读一篇；`admin reads` 查到该次读取。

## 9. 部署形态（T-034，2026-09-07）

- `serve` 只监听 `127.0.0.1:<port>`，不读 Host/Origin：公网入口由专用 Cloudflare Tunnel 的独立主机名转发到回环端口并把 Host 覆写为 `127.0.0.1:<port>`；应用不信任任何转发头，身份只来自 bearer 类型 token。
- 优雅关停：SIGINT 与 SIGTERM 等价（systemd 默认 `KillSignal=SIGTERM`）；关停期间不再接受新会话，已开始的供给完成后退出。
- T-034 已部署服务器使用 `--vault` 读取原只读副本。T-036 新发布模板使用 `--replica-root`；先完成首批完整同步，再按运维步骤更换 MCP 单元。两种形态白名单均为 `项目/Studiolo`、`_规则`，身份、审计与预算要求相同。
- 未加 Cloudflare Access：服务 token 秘密不能经 agent 之手；主人在面板签发后可再挂 Access 应用（policy 为 Service Auth），届时客户端多带两条 `CF-Access-Client-*` 头即可，本契约不变。
- 服务器安装、目录约定与实际部署记录见 `docs/m3/server-agent.md`。
