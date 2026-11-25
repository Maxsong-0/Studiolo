# 连接器入口与「在做什么」状态口契约（`/connect/v1/*`、`GET /api/v1/board/presence`）

T-237，2026-11-24；决定见 vault `03-记忆/ADR-046-连接器入口窄能力集和在做什么状态口.md`。给只有窄权限的机器程序
（第一个用户是 HomeLab Center 的平台连接器）：一张令牌只看黑板，另一张只推「谁的会话在做什么」，读不到别的。
状态**不是工作项**：不进控制库、不进备份、不投影到 `TASKS.md`，不参与认领、编号、回执，也不能转成工作项。

实现：agent 那一半 `crates/agent-server/src/connector.rs`（`connector/{limits,reply}.rs`），人侧那一半
`crates/local-server/src/board/{connector,presence}.rs`，能力集校验 `crates/policy/src/validate/connector.rs`，
界面 `web/src/{presenceModel,presenceApi}.ts`、`web/src/cockpit/{presence.ts,usePresenceRead.ts,PresenceSection.tsx}`。

## 1. 权限：两份窄名单

| 能力集字段 | 准做什么 |
|---|---|
| `read.board_projects` | 看这些项目的黑板（列表元数据、单条全文） |
| `write.presence_projects` | 推这些项目上的会话状态 |

- 两个字段都可不带；不带时能力集正文和以前一字不差。旧程序读到带新字段的正文，只有那一类型报策略库损坏，别的类型照常。
- 发布时拦：值是完整项目路径（`项目/X-Docs`，≤512 字节、没有看不见的字符）、不重复、不是受保护目录；**任一个非空时，
  这份能力集别的权限必须全空**，两个也不能同时非空。名单加项目算扩大，照常标红、照常要主人批。
- **连接器的名单不能清空、也不能改成普通能力集**：清空后它就是一份普通的空能力集，令牌反而开得了 MCP。要停就撤下这份能力集，
  或吊销类型。
- 名单不和 `--allow` 求交：`--allow` 管的是供给副本，连接器不经副本，AI 副本过期也不影响这两个口子。
- 名单非空的类型（连接器类型）进不了别处：`/read/*` 一律 **403**；MCP 握手照现有策略拒绝的写法——应答 200、说明文字是
  「策略拒绝：该类型只能走连接器入口…」、没有路牌、没有任何内容，之后每次调用都是同一句拒绝。两条都在
  `server/grant.rs` 同一处判。

## 2. 共同规矩

- 一次请求就是一次完整认证：`Authorization: Bearer <类型令牌>`，没有会话、不握手、不发路牌、不续期。
- **每个答复都是 JSON，带 `Studiolo-Connect: 1` 和 `Cache-Control: no-store`**（304 没有正文，头照带）。没有这个头、
  或带 `cf-mitigated`，就不是 Studiolo 回的（边缘质询、隧道或网络）：客户端写「被 Cloudflare 拦了」并退避，
  不当成令牌错，也不当成黑板空；不解质询。客户端要有自己的 `User-Agent`。
- 限速：每类型每入口 12 次／分，全进程同时 4 个（含排队，其中同时交给人侧的 2 个；人侧的 4 个黑板名额连接器最多占 2 个）；看黑板间隔 ≥ 30 秒，推状态每 30 秒一次、变了可以马上推但间隔 ≥ 5 秒。
- 失败正文固定是 `{"code","error"}`，不回显输入、路径和内部原因：

| 状态码 | `code` | 什么时候 | 客户端怎么办 |
|---|---|---|---|
| 400 | `invalid_request` | 多了查询参数、缺参数、正文不是 JSON 对象 | 改程序 |
| 401 | `unauthenticated` | 令牌错、已撤（带 `WWW-Authenticate: Bearer`） | 停下，等人换令牌 |
| 403 | `not_allowed` | 类型没有这个入口的名单，或被吊销 | 停下，等人放行 |
| 404 | `not_found`／`no_such_entry` | 没有这条（名单外与不存在同一句话）／没有这个入口 | — |
| 405 | `method_not_allowed` | 方法不对 | 改程序 |
| 408 | `body_timeout` | 快照正文 10 秒没收全 | 重推最新一份 |
| 409 | `stale_seq` | `seq` 不比上一份大 | 用更大的 `seq` 重推最新一份 |
| 413 | `too_large` | 快照超过 64 KiB | 改程序 |
| 422 | `invalid_sessions` | 字段不合，整份没收、上一份不变 | 改程序 |
| 429 | `too_many_requests` | 太快，或同时太多 | 等 `Retry-After` 秒 |
| 503 | `unavailable`／`stopping` | 这一刻给不出结论 | 等 `Retry-After` 秒再试 |

  503 和 401、403 不混：身份库、策略库读不出来一律 503（BUG-030）。agent 账本被备份封存冻结着时，轮到它之后
  最多再等 5 秒（前面排着的各等各的，4 个同时来时最后两个约 10 秒），等不到一律 503：冻结中不登记实例、不改最近活动、不打人侧。

## 3. 看黑板

### `GET /connect/v1/board`

```json
{"generated_at": "RFC3339", "projects": [
  {"project": "项目/Studiolo-Docs", "items": [], "finished": 3, "truncated": false}]}
```

- 没放行的项目不出现；放行了没工作项是 `"items": []`。按项目路径排序。
- `items` 只列没结束的，字段与[跨项目黑板汇总](board-overview.md)第 2 节的工作项相同：只有元数据，没有正文、指令、
  验收标准、回执正文。`finished` 是结束了的条数。
- 每个项目最多 200 条、整份正文最多约 700 KiB，收住了就 `truncated: true`。
- 带 `ETag`（只摘 `projects`，不含 `generated_at`）。`If-None-Match` 命中回 **304**；边缘压缩改成弱标签（`W/"…"`）照样认。

### `GET /connect/v1/board/task?project=项目/X-Docs&task_id=<ULID>`

单条全文，和 `board` 工具的 `get` 一样的 `BoardView`。名单外、项目对不上、不存在都是同一个 404。**正文交出去就算离开
Studiolo**：对方用不上就别取，取了只在内存里留。

## 4. 推状态：`PUT /connect/v1/sessions`

整份快照替换，不做逐条增删：快照里没有的当场消失，`"sessions": []` 就是清空。**满 300 秒没有新快照，这个来源整份消失。**
`Content-Type: application/json`。

```json
{"seq": 1795578309925, "sessions": [{"kind": "session", "key": "9f…（随机）", "vendor": "claude_code",
  "project": "项目/Studiolo-Docs", "branch": "claude/t-237", "state": "running",
  "step": 3, "total": 8, "last_action": "edit", "idle_secs": 12}]}
```

| 字段 | 必带 | 规矩 |
|---|---|---|
| `seq` | 是 | 0～2^53−1，比上一份大；用毫秒时间最省事。没有上一份（人侧重启、过期、换了令牌之后）时任何值都收 |
| `kind` | 是 | 只收 `session`（以后 Bottega P4 加 `card`） |
| `key` | 是 | 16～64 位小写十六进制，自己生成的随机编号，**不能是真的会话号**；同一份里不重复；读侧不给 |
| `vendor` | 是 | `claude_code`／`codex`／`agy` |
| `project` | 是 | 完整项目路径；不在名单里的单条拒，别的照收 |
| `state` | 是 | `running` 在跑／`waiting` 等你／`idle` 做完或等你回话 |
| `branch` | 否 | 1～120 字节的 git 引用名；控制字符和看不见的字符不收（格式、双向、零宽、变体选择、填充符这些，全表在 `crates/policy/src/visible.rs`）；像密钥的串不收（对方先挡一道，这里是第二道） |
| `step`、`total` | 否 | 0～999；`total` 只能跟着 `step` 来、且不小于它 |
| `last_action` | 否 | `read`／`edit`／`run`／`web`／`ask`／`dispatch`／`other`；工具名不收 |
| `idle_secs` | 否 | 0～86400；读侧按收到的时刻往上加 |

- 可不带的缺了或写 `null` 都当没有；取不到就不带，不收「未知」这种占位。多一个字段、取值不对，整份 422、上一份不变。
- 每个来源 64 条，全部来源合计 256 条。
- 回 `{"accepted": 2, "rejected": [{"index": 1, "reason": "project_not_allowed"}], "expires_in_secs": 300}`；`rejected` 按下标
  排，原因只有两种：`project_not_allowed` 项目不在名单，`too_many_new` 这一小时「新会话」的登记用完了（老编号照收照更新，
  下一小时名额陆续回来；不用停推，也不用换 `seq`）。
- 连不上只留最新一份，不补发旧的。

## 5. 读状态

| 谁 | 从哪读 | 门 |
|---|---|---|
| 主人 | `GET /api/v1/board/presence`（驾驶舱「谁在干活」里的「本机会话」） | 登录会话；不记审计；没装黑板 404；忙 429；给不出 503；多一个查询参数 400 |
| 别的 AI | `board` 工具的 `presence` 操作 | 和 `list` 同一道项目白名单，先审计后给，只给请求的那个项目 |

```json
{"since": "RFC3339", "expires_in_secs": 300, "sessions": [{"source": "homelab-status", "vendor": "claude_code",
  "project": "项目/Studiolo-Docs", "branch": "…", "state": "running", "step": 3, "total": 8,
  "last_action": "edit", "idle_secs": 20, "age_secs": 8}]}
```

- `since` 是内存从几点起攒的：人侧重启后是空的，靠它分得清「没有」和「还没推来」，下一次推就回来。没带的字段不出现。
- **撤了就没**：每次读先按此刻的授权收窄。令牌换了一枚、类型或实例被吊销、能力集撤下或收回一个项目，下一次读就没有那些条，不等 300 秒；
  授权读不出来就一条不给。15 秒一轮的巡检顺手清。
- 前端封闭解码（多一个字段、取值不在枚举里整份作废），读失败只影响这一组、不留旧状态；一次读取最多等 8 秒，没有结果的读取也不留旧的：
  每条按自己的岁数在本地到期，原来有的全到期了、或整份 300 秒没重读成，就换成「上次读到的已经过期」（不说成「没有」，也不说成「读不到」：读取可能刚成功过），下一次重读给结论；不计入「等你处理」的巨号和角标。

## 6. 审计

| 动作 | 记在哪 |
|---|---|
| 看列表 200 | agent 账本每个项目一条 `content_read`（工具名 `connect.board`），先记后给；304 不记 |
| 看单条 | agent 账本一条 `content_read`（`connect.board.task`） |
| 推状态 | 人侧账本：只在「新编号第一次出现」时一条 `board.presence`（类型、实例、项目、哪家、条数；同项目同一家并成一条）。**更新、过期、消失不记**。先登记后落下，登记不了整份不收（503） |
| 拒绝 | 同一（类型，原因）每分钟最多一条，没认出类型的合在一起算（动作 `connect.request`；和 `/mcp`、`/read/*` 各算各的，限频是同一份实现）；登记不了不改变拒绝的结论 |

- 新会话的登记每个来源每小时最多 60 条（同项目同一家并成一条），用完了多出来的新编号逐条拒（第 4 节）：账本不能被灌。
- 封存冻结着审计时：含新编号的整份 503，只含老编号的照收。
- 每个连接器类型在 agent 进程每次起来后登记一个实例（登记的审计写不进就先不能用，下次只补这条、不再另登记），「接入方」页看得到它最近一次活动；实例级吊销对它也管用。

## 7. 权限判两遍

agent 先按令牌、三级吊销、能力集判一遍，连同这枚令牌的签发时刻交给人侧；人侧在私有 Unix 入口 `POST /internal/connector`（和 `/internal/board` 同一道
同 UID 门）再按身份库、策略库判一遍（签发时刻和身份库现值不同就是已经换过令牌，拒），项目过滤在人侧做，agent 只核「没多给」。交付前两边各再复核一次：排队、等锁期间撤了权，
一项都不给。边界由架构守卫 `crates/studiolo/tests/architecture_cases/connector_entry.rs` 钉住。

## 8. 上线核对与回滚

- `scripts/check-connector.py <地址> <token 文件> board|status`：在 169 回环和对方机器上各跑一次。`status` 只推一份故意不合的
  快照（预期 422），不会动现有状态。退出码 0 通过、1 不符、3 被边缘拦了；不走环境代理。本机回环上缺标记头说的是
  「agent 没有这个入口（旧发行）」，按不符报。
- 当场断：吊销类型、撤下能力集、或 `type-revoke`（换一枚令牌也算）；状态当场没有，不留痕迹。
- 退发行之前先撤下两份能力集（旧程序读不懂新字段）；没有迁移要退，控制库 schema 仍是 4。
- 新 agent 配旧人侧：`/connect/*` 回 503；旧 agent 配新人侧：没有这些入口。都不影响别的功能。

## 9. 验证边界

自动测试：`crates/policy/tests/connector_capability_contract.rs`、`crates/local-server/src/board/presence/tests.rs`（假时钟）、
`crates/local-server/tests/board_http/connector.rs`、`crates/agent-server/tests/policy_cases/connector.rs`（假权威服务）、
`web/src/test/{presenceModel.test.ts,cockpit/presence.test.tsx}`、`scripts/test-check-connector.py`；
脚本不走环境代理由 `scripts/test-no-environment-proxy.py` 扫全部脚本。

未覆盖：真的 Cloudflare 边缘、HomeLab 那一侧的连接器、生产规模的黑板列表耗时、两个真进程对接的长时间运行；这些在上线那一步核。
