# 策略平面契约（M4，2026-09-17 T-069）

> 权威依据：vault [[ADR-010-内容平面写者归属]] 一、[[ADR-015-分发外壳与AI身份]] 三／四、
> [[ADR-019-策略平面存储与写者归属]]、[[PRD-v2-10-附录]] 22.2、[[敏感动作枚举表]] 与子档
> [[T-069-策略平面动作登记]]、[[roadmap]] M4 行。本契约只描述**策略平面**：规则母本、skills、
> 各 AI 类型的能力集三类母本的 desired → 校验 → diff → L3 发布 → effective，版本与回滚，
> 实例／类型／全局三级吊销，以及接入方如何被它约束。黑板写入（M5）与执行体（M6）不在此。

## 1. Module 与装配

| crate | 角色 | 依赖（workspace 内） |
|---|---|---|
| `studiolo-policy` | 纯库：`policy.sqlite3` 私有库、三类母本的形状与校验、diff 与扩大项、版本／回滚／bootstrap、三级吊销、两个句柄、审计事件构造 | core、audit-ledger、rusqlite |
| `studiolo-local-server` | 人侧：`--policy-data` 打开 `PolicyStore`，`/api/v1/policy/*` 与 `/api/v1/agents/revoke`，发布挡在 `FactorRuntime::require_l3(ActivatePolicy)` 之后 | ＋policy |
| `studiolo-agent-server` | 接入方：`serve --policy` 打开 `PolicyAgentStore`，每次调用复核，供给视图求交，`policy_read`／`propose_desired`／`list_proposals`，admin 子命令 | ＋policy |

- 私有目录：绝对路径、0700；唯一文件 `policy.sqlite3`（0600，单链接，同 owner，`-wal/-shm/-journal` 同规则）。
  架构守卫登记第六个固定私有库文件名，`Connection::open_with_flags` 只出现在 `crates/policy/src/database.rs`。
- 格式 1：`application_id` = ASCII `SPOL`，`user_version` = 1，五张表（`policy_version`／`policy_current`／
  `policy_desired`／`policy_revocation`／`policy_meta`）与四个索引精确比对，缺表、多表、头不符、`quick_check`
  或外键失败一律 `CorruptStore`，不隐式迁移。
- 两个句柄打开同一个文件：`PolicyStore`（主人发布者：唯一能写版本表与吊销表）与 `PolicyAgentStore`
  （接入方视角：读 effective／吊销状态，只能向 `policy_desired` 插入 `pending` 提案）。提案永不产生授权。
- `serve` 未给 `--policy` ＝ **M3 兼容模式**：供给视图只来自 `--allow`，路牌第 3 项明写「策略平面：未接入」，
  `handshake.served` 记 `policy_plane=absent`。**`tools/list` 与是否给 `--policy` 无关，恒为八个**（四个只读 ＋ 黑板 `board` ＋
  `policy_read`／`propose_desired`／`list_proposals`，T-182 起由七个变八个）；
  兼容模式下这三个策略工具一律以 JSON-RPC `-32600`、固定文案「策略平面未接入（M3 兼容模式）」拒绝并记 `agent.action_denied{policy_plane_absent}`。
  上线探针 `scripts/check-agent.py` 逐字依赖这三点（路牌标记、八工具清单、拒绝码与文案），三处由
  `crates/agent-server/tests/policy_contract.rs` 的兼容模式用例钉住；改任何一处必须同步改探针。

## 2. 三类母本

| kind | subject | 正文 | 规范化 |
|---|---|---|---|
| `rules` | 单段文件名，`.md` 结尾，NFC，≤128 字符，不以 `.` 开头 | Markdown | CRLF→LF；拒绝 NUL 与空文 |
| `skills` | `[a-z0-9-]{1,64}`，不以 `-` 开头 | JSON `{name, summary, triggers[], markdown}`（`deny_unknown_fields`） | 解析后重新序列化；`name` 必须等于 subject |
| `capability_set` | AI 类型名 `[a-z0-9-]{1,32}` | JSON（PRD-v2-10 §22.2 形状，`deny_unknown_fields`） | 解析后重新序列化；`ai_type` 必须等于 subject |

能力集形状：
```json
{"ai_type":"claude",
 "read":{"project_whitelist":["项目/Studiolo","_规则"],"deny_by_default":true,
         "rules_whitelist":["通用工作规则.md","CLAUDE.md"]},
 "write":{"board":false,"content_patch_only":true,"content_direct":false},
 "policy":{"propose_desired":true,"activate":false},
 "hard_bound_skills":[{"when":"写或改 React / Next.js 代码","must_use":"vercel-react-best-practices"}]}
```
`version` 与 `policy_version_hash` 由发布事务盖章，提交方不填。**恒定约束，不可由配置放宽**：
`read.deny_by_default=true`、`write.content_direct=false`、`write.board=false`（M5 前）、`policy.activate=false`；
白名单每项须是合法逻辑路径前缀且不指向受保护目录；`hard_bound_skills[].must_use` 必须指向已发布的 skill；
被硬绑定的 skill 不能撤销。正文上限：主人 256 KiB，接入方提案 64 KiB。

**规则名单 `read.rules_whitelist`**（T-160，vault ADR-026 D3／D4）：该类型可读的规则 subject 列表，每项须满足
rules 类的 subject 规则（`.md` 结尾、≤128 字符、不含 `/`、不以 `.` 开头），不得重复；不合规报
`read.rules_whitelist[i]`「不是合法的规则 subject」，重复报「重复的规则 subject」。字段 `#[serde(default)]`：
**缺字段即零规则**（失败关闭），旧能力集照常解析；规范化正文恒写出该字段（缺省为 `[]`）。名单只管接入方，
人侧永远全读。规则名单管的是 rules 类 effective 与 vault `_规则/` 下的规则文件，两者用同一判定，见 §5。

## 3. desired → 校验 → diff → 发布 → effective

- **提交**（主人 `POST /api/v1/policy/desired`；接入方 MCP `propose_desired`）：入库的是提交原文；同事务内对当前
  effective 做校验与 diff，报告存进该 desired。主人提交记 `validated`／`rejected`，接入方提交记 `pending`（结构无效则
  `rejected`）；两者都**永不自动生效**。接入方每实例最多 8 条 pending、全库 256 条。
- **接入方断线重提**（T-182）：**本类型**在同一 kind／subject 上**最新**的未发布草案（pending／validated／rejected，按入库
  先后）若正是规范化正文摘要相同、仍待处理（pending／validated）的那份，原样返回它（`deduplicated=true`），不新建、不占名额；
  判定在名额检查之前、与入库同一写事务。否则照常有界入库：本类型之后又提过别的正文，再提旧正文就新建一条，让它重新成为本类型
  最新的提议。只看本类型自己的草案，与发布的拦截同一口径（同一提交者有更新的未发布草案才挡，T-181）：主人和别的类型的更新
  草案既不挡本类型那份的发布，也不让重提多出一条；重提的结果也就不透露别人有没有新草案。
  按类型而不按实例判定，因为断线重连后是另一个实例；规范化失败（结构无效）的正文没有摘要，每次都新建一条 `rejected`；
  能规范化、语义被拒的有真摘要，同样不拿回（重提要再校验一次）。
  调用方看到错误时提案有没有入库：会话层门禁（`crates/agent-server/src/mcp_transport/gate.rs`）回的 404（会话查不到或已
  过期）与 400（缺会话号）只发生在交给 SDK **之前**，说明工具没有执行；401、503 在交给 SDK 前后都可能回（凭据被吊销、身份库
  读不出、服务正在关停、会话途中结束），再加上 408 超时、工具末尾 `finish_mcp` 报会话无效、入库后审计不可用、响应送达前连接
  断开，这些都说明不了有没有入库。拿不准就先用 `list_proposals` 核对，或直接重提。重提只在那份提案**仍是本类型在该 subject 最新的待处理草案**时
  拿回它，并补记一条 `policy.desired_submitted`（`deduplicated=true`，主体是重提的实例）；本类型之后提过别的正文，或它已被
  发布／撤回／取代时，重提照常新建，第一次提交没记上的那条审计不会补；只核对、不重提的也不补（提案无授权效果，人侧照样看得到它）。
- **报告**：`ok`、逐条 `problems{path,message}`（不回显正文）、`diff`（JSON 字段级或 Markdown 行级 LCS，≤2000 条，超出标
  `truncated`）、`expansions`（扩大项，界面标红）、`narrowings`、`base_effective_hash`、`current_version`、`body_sha256`。
- **扩大项判定**：能力集——白名单新增或变宽、规则名单新增（`read.rules_whitelist`：新类型记「新类型首次获得规则读取」，
  已有类型记「规则读取范围变宽」；移除项记收窄「规则读取范围收窄」）、`content_patch_only`／`propose_desired` false→true、硬绑定被解除、首次授予；
  skills——被硬绑定的 skill 正文变更（标「人工判断」）；rules——被删除的含「禁止／绝不／不得／一律／必须／永不／必叠」的行
  （候选，人工判断）；撤销整份母本恒为扩大项。
- **发布**（`POST /api/v1/policy/activate`，L3）：请求三选一 `{desired_id}` / `{rollback_to}` / `{release_global_revocation:true}`，
  必带 `expected_effective_hash`（CAS）。服务端固定顺序：重验会话 → 审计 **Intent**（`policy.activated`）→
  `require_l3(ActivatePolicy)` → 策略库事务（哈希 CAS → 只认同一对象最新的未发布 desired → 重新校验 → 写版本 → 更新现行 →
  重算 effective 哈希 → 取代同 subject 其余（都比它早的）未发布 desired → 能力集发布顺带解除该类型吊销）→ 审计 **Outcome**（同 id）。
  拒绝一律 `policy.activate_denied{reason}`：`reauthentication_required`（403）、`no_l3_path`（409）、`untrusted_source`（403）、
  `hash_mismatch`（409，回传 `current_effective_hash`）、`newer_desired_exists`（409，回传 `newer_desired_ids`）、
  `validation_failed`（422）、`not_found`（404）、`not_activatable`（409）。
  **只能发布同一对象最新的未发布 desired**（T-181，BUG-031）：同一 kind/subject 还有比它晚入库的 pending／validated／rejected
  就拒绝，什么都不写；要发较早那份，先撤回更新的。「谁更新」按入库先后（SQLite rowid）判，不看提交时间、不看草案号；
  草案列表 `GET /api/v1/policy/desired` 同时回 `newer_open`（每份未发布 desired → 比它晚入库的未发布 desired 号，按入库先后，
  最新那份是空表），与拦截同一口径（`crates/policy/src/store/queue.rs`），界面只照它标「最新」「较早」、挡发布按钮。
  列表的 200 份只截已结束的历史：未发布 desired 一份不少，落在窗口外的按同一排序补在末尾。判定找不到要发的那份
  （行与判定对不上）就报库损坏（503 `store_unavailable`），不放行。
  账本不可用即失败关闭不发布；Outcome 落账失败时发布已持久，响应 `audit_confirmed=false` 而不是假装没发布。
- **effective 哈希** ＝ 全部现行 (kind, subject, version, body_sha256) 按序拼接的 SHA-256；空策略也有确定值。
  每个版本记 `effective_hash`／`previous_effective_hash`；`handshake.served` 记接入方每次拿到的 `effective_hash`，
  与 `policy.activated` 链逐条可对账（PRD-v2-01 2.4 第 7 条）。
- **回滚** ＝ 新建一个正文等于历史版本的版本（`rollback_of`），历史不变；回滚目标须通过当前校验（例如引用了已撤销的 skill 则拒绝）。
- **bootstrap**（`studiolo-agent admin policy-bootstrap`）：真实前台控制终端门禁；只允许为从未有过版本的 subject 写 version 1；
  记 `policy.bootstrapped`。之后一切变更走人侧 L3。

## 4. 三级吊销

| 级别 | engage | 生效 | restore |
|---|---|---|---|
| 实例 | `POST /api/v1/agents/revoke{level:"instance",target}`（L2） | 下一次调用即拒（`instance_revoked`） | 不可恢复 |
| 类型 | 同上 `level:"type"`（L2）；或 `admin type-revoke`（身份库） | 全部实例下一次调用即拒（`type_revoked`）；新握手拒绝 | 重新发布该类型能力集（L3）自动解除策略库的类型吊销；身份库吊销由 `type-issue` 轮换恢复 |
| 全局 | `level:"global"`（L2） | 全部拒绝（`global_revoked`） | `activate{release_global_revocation:true}`（L3） |

复核顺序固定：全局 → 类型 → 实例 → 能力集；同一读事务内取值，无缓存窗口。审计：`agent_instance.revoked`／
`agent_type.revoked`／`agent.global_revoked`（吊销先生效再补记，账本不可用不撤回）、`agent.global_released`、`agent_type.restored`。

## 5. 接入方视角

- **供给视图 ＝ `--allow`（部署上限）∩ 能力集 `project_whitelist`**，取较窄前缀；没有能力集的类型是**零权限**，不是空视图。
  视图内直接位于 `_规则/` 下的 `.md` 规则文件另须在 `read.rules_whitelist` 内（ADR-026 D9／D10）。
- 握手被策略拒绝时**应答但一项不给**：`initialize` 返回 `instructions = "策略拒绝：<原因>；本次会话所有读取与写入全部拒绝"`，
  路牌六项一项不给；每次调用再次复核并记 `agent.action_denied{reason}`。HTTP 文本外壳映射为 403。
- 路牌改自 effective：第 3 项「读：<视图>（能力集 v<n> ∩ 部署上限）；写：…；策略：…；策略版本 <哈希 12 位>」；
  第 5 项准则取策略平面规则母本（无则退回 vault 文件；两者都只取规则名单内的），硬绑定标「（策略强制）」；
  第 6 项加 `policy://rules/<subject>@v<n>`（只列名单内的）与 skills 计数。仍受 ≤2000 token 硬预算。
- `policy_read{kind=rules|skills[,subject]}`／`GET /read/policy`：读 effective 规则母本或 skill；每次先追加 `content.read`
  （路径 `policy://…`，revision `v<n>`，tool `policy_read`）并进四维读取索引。
  首行 `# 策略版本 <12 位>` 与清单／正文取自同一个 SQLite 读事务（`PolicyAgentStore::rules()`／`skills()` 返回
  `AtEffective`），人侧恰在读取途中发布也不会让版本行与条目各属一版（T-134）。路牌第 3／5／6 项同理用
  `rules_and_skills()` 一次读出，且其哈希须等于本次授权复核时的哈希，不等即失败关闭让调用方重试。
  输出形状：规则清单逐行 `subject\tv<n>\t<sha256 前 12 位>`，单条为 `policy://rules/<subject>@v<n> sha256 <64 位>`
  行后接正文原字节；skills 清单逐行 `名\tv<n>\t摘要\t触发（以 " | " 相连）`，摘要与触发里的控制字符与 U+2028／U+2029
  压成空格，保证一个 skill 恰占一行；**单条 skill 的正文是入库的规范 JSON 原文**（`{name,summary,triggers,markdown}`，
  serde 字段序、紧凑分隔），sha256 行就是这段字节的摘要（T-160，待拍板清单第 9 条选 (a)）。
- **规则级读权限**（T-160，vault ADR-026 D5）：`policy_read kind=rules` 的清单与单条、路牌第 5／6 项的策略规则，
  都只给能力集 `read.rules_whitelist` 里有的 subject；vault `_规则/` 下规则文件的五条读取路径用同一判定，见
  [供给面契约](agent-supply.md) §10。清单里名单外的规则不出现（计数随之一致）；单条读名单外的 subject 对外与不存在
  同一句（MCP `resource_not_found`「规则母本不存在」，HTTP 404），审计 `agent.action_denied{reason=rule_not_whitelisted}`，
  名单内而不存在才记 `not_found`。名单取自本次授权复核时的能力集：规则读取所见的 effective 哈希须与复核时相同，
  不等即失败关闭让调用方重试（与路牌同一做法）。
- **skill 硬绑定与索引分家**：硬绑定＝能力集 `hard_bound_skills[{trigger, must_use}]`，策略强制，进路牌第 5 项并标「（策略强制）」；
  索引＝`policy_read{kind=skills}` 不带 `subject` 时返回全部已发布 skill 的「名称／版本／摘要／触发词」清单，只是候选、由 AI 自选，
  带 `subject` 才取正文。**Studiolo 不做语义路由**：不替 AI 判断该用哪个 skill，只保证硬绑定那几条一定出现在路牌里。
- `propose_desired{kind,subject,body}`：需能力集 `policy.propose_desired`；入库 `pending`，记 `policy.desired_submitted`
  （接入方的多一个 `deduplicated`）；返回 id／状态／扩大项计数／`body_sha256`（服务端算的规范化正文摘要，规则母本 CRLF
  换 LF 后；过不了规范化为 `-`）／`deduplicated`／逐条问题；断线重提见 §3；**不改变 effective**。
- `list_proposals{kind?,subject?}`（T-182，只走 MCP）：供断线后核对提案有没有入库。门槛与 `propose_desired` 相同（未接入策略
  平面同样拒绝并记 `policy_plane_absent`，无提案能力记 `capability_denied`）；只列**本类型**仍待处理（pending／validated）的
  接入方提案，新到旧，最多 64 份（超出时首行注明还有更早的）；每行 `desired_id`、`kind`、`subject`、状态、`body_sha256`、
  提交时间、实例，以制表符分隔，不含正文与报告；先记 `content.read{path=policy://desired, revision=-, tool=list_proposals}`
  并进读取索引，再返回。

## 6. 错误模式

| 错误 | 触发 | 对外 |
|---|---|---|
| `InvalidKind` / `InvalidSubject` / `InvalidBody` | 类别、subject 或正文不合规 | 400；入库前拒绝，不留记录 |
| `ValidationFailed` | 发布时重新校验未通过 | 422 ＋ desired 报告更新 |
| `HashMismatch{current}` | CAS 失败 | 409 ＋ `current_effective_hash` |
| `NewerDesiredExists{newer}` | 发布的 desired 不是同一对象最新的未发布 desired（T-181） | 409 `newer_desired_exists` ＋ `newer_desired_ids`；什么都不写 |
| `DesiredNotFound` / `VersionNotFound` | 不存在 | 404 |
| `DesiredNotActivatable` / `NotRevoked` / `AlreadyBootstrapped` | 状态不允许 | 409 |
| `TooManyProposals` | 接入方提案超界 | MCP 错误「待处理提案已达上限」 |
| `UnsafeStorage` / `MissingStore` / `CorruptStore` / `StoreUnavailable` | 私有库 | 503；启动期直接拒绝启动 |
| （路牌、`policy_read` 规则）授权复核与条目读取之间 effective 变了 | 人侧恰在一次调用途中发布 | MCP 错误「策略在本次调用途中发布了新版本，本次拒绝供给，请重试」 |
| `rule_not_whitelisted` | 接入方读名单外的规则母本或 `_规则/` 规则文件 | 与不存在（`policy_read`）或白名单外（`read_note`）同一对外错误，只在审计里区分 |

## 7. 契约测试清单

- `crates/policy/tests`：校验／diff／扩大项、CAS 发布、恒定约束不可放宽、硬绑定阻止撤销、规则约束行删除标红、回滚、
  bootstrap 首版限制、三级吊销与恢复方向、接入方提案有界且永不生效、原文原样保留、多表／错头即损坏、视图求交、审计事件形状；
  `agent_read_snapshot_contract.rs`：接入方读取与策略版本同一读事务，并发发布 40 版下无一撕裂（改回两个事务即红）；
  `rules_whitelist_contract.rs`：旧能力集缺字段解析为空、合法名单通过、非法与重复项按下标报问题、新增报扩大、移除报收窄。
- T-181 同一对象几份未发布 desired：`crates/policy/tests/desired_order_contract.rs`（只能发布最新那份，较早的被拒、什么都不写；被拒的更新草案也挡，撤回后放行；按入库先后、不看时间戳；`desired_queue` 与拦截同一口径，历史窗口不漏未发布草案；判定对不上报库损坏），`policy_http_contract.rs`（409 `newer_desired_exists`、审计原因、列表 `newer_open`），前端 `web/src/test/policy/newerDraft.test.tsx`、`web/src/test/rules/draftPanel.test.tsx`（标「最新」「较早」、挡较早草案、预览写出草案号与取代）、`policyStepUp.test.tsx`（认证期间来了更新的草案，重试被 409 挡下）、`policyModel.test.ts`（`newer_open` 必须是同一对象内的一条先后，自相矛盾整份拒收）。
- `crates/agent-server/tests/rules_access_contract.rs`：规则名单对 `policy_read` 清单与单条、握手路牌、`read_note`／
  `list_entries`／`search` 同一裁决，名单外与不存在（或白名单外）对外同一错误、审计 `rule_not_whitelisted`，
  `_规则/` 子目录照项目白名单，缺字段零规则，M3 兼容模式不过滤；`crates/supply/tests/rules_files_contract.rs`：
  规则文件判定与路牌库文件兜底按名单裁决。
- `crates/agent-server/tests/policy_contract.rs`：零权限握手、供给视图求交、路牌 3／5／6 项、`policy_read` 审计与索引、
  `propose_desired` 永不生效与能力否决、三级吊销即刻生效与恢复、HTTP 外壳同一授权、M3 兼容模式。
- T-182 断线重提：`crates/policy/tests/proposal_contract.rs`（CRLF／LF 与规范 JSON 视为同一份、名额满时重提拿回原提案、
  只有本类型最新的草案接得住重提、主人或别的类型的更新草案不影响重提、rejected（含语义被拒、有真摘要的）／撤回／主人提交的不接、
  跨类型不相认、主人句柄的接入方分支同一规则、多条连接同时重提只留一条、列表的范围／顺序／上限、发布与取代后出列、
  主人复核超长提案后摘要列同步）；`crates/agent-server/tests/proposal_contract.rs`（换实例重提只有一条且返回同一 id 与摘要、
  语义被拒也给真摘要且不去重、入库后审计不可用如实说已入库且恢复后重提补记、`list_proposals` 只列本类型、进读取审计与索引、
  审计不可用时失败关闭、无能力／三级吊销／未接入时拒绝、工具说明里的份数与常量一致）。
- `crates/local-server/tests/policy_http_contract.rs`：真实 Unix 入口＋TOTP：发布挡在 L3 后（用例 36 人侧）、Intent/Outcome
  同 id、CAS 409、回滚、版本读取、三级吊销 L2 与全局解除 L3、方法与查询封闭、未装配策略平面 404。
- 前端 `web/src/test/policy*`：模型封闭解析、扩大项标红、吊销提交、未接入显示；发布遇 403 `reauthentication_required` 就地弹二次认证窗口，认证通过后**按原 `expected_effective_hash`** 自动重试同一次动作（取消或认证失败都不重试；期间 effective 变了仍按原 hash 提交、由服务端 `hash_mismatch` 挡下；本入口无 One-time Code 路径时窗口直说不能发布）。

## T-174 规则差异与扩大项完整性

规则按完整增删事件流判定约束删除，独立于最多 2,000 条的展示 diff；
展示截断仍须保留末尾的约束删除证据。扩大项逐条最多展示 2,000 条，超过时另加一条
明确的遗漏数量与完整母本审阅提示，不能把未展示解释为没有扩大。
LCS 先裁共同首尾，仅对变化区间分配一块连续表，预算包含边界行列（最多 4,000,000 格）。
空侧直接产生事件，超预算的变化区间标记整体替换；相同的大母本不误报整份删除/新增。
发布重新校验与草案审阅使用同一路径，保留原始一基行号。
