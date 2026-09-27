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
| `studiolo-agent-server` | 接入方：`serve --policy` 打开 `PolicyAgentStore`，每次调用复核，供给视图求交，`policy_read`／`propose_desired`，admin 子命令 | ＋policy |

- 私有目录：绝对路径、0700；唯一文件 `policy.sqlite3`（0600，单链接，同 owner，`-wal/-shm/-journal` 同规则）。
  架构守卫登记第六个固定私有库文件名，`Connection::open_with_flags` 只出现在 `crates/policy/src/database.rs`。
- 格式 1：`application_id` = ASCII `SPOL`，`user_version` = 1，五张表（`policy_version`／`policy_current`／
  `policy_desired`／`policy_revocation`／`policy_meta`）与四个索引精确比对，缺表、多表、头不符、`quick_check`
  或外键失败一律 `CorruptStore`，不隐式迁移。
- 两个句柄打开同一个文件：`PolicyStore`（主人发布者：唯一能写版本表与吊销表）与 `PolicyAgentStore`
  （接入方视角：读 effective／吊销状态，只能向 `policy_desired` 插入 `pending` 提案）。提案永不产生授权。
- `serve` 未给 `--policy` ＝ **M3 兼容模式**：供给视图只来自 `--allow`，路牌第 3 项明写「策略平面：未接入」，
  `handshake.served` 记 `policy_plane=absent`。**`tools/list` 与是否给 `--policy` 无关，恒为六个**（四个只读 ＋ `policy_read`／`propose_desired`）；
  兼容模式下这两个策略工具一律以 JSON-RPC `-32600`、固定文案「策略平面未接入（M3 兼容模式）」拒绝并记 `agent.action_denied{policy_plane_absent}`。
  上线探针 `scripts/check-agent.py` 逐字依赖这三点（路牌标记、六工具清单、拒绝码与文案），三处由
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
- **报告**：`ok`、逐条 `problems{path,message}`（不回显正文）、`diff`（JSON 字段级或 Markdown 行级 LCS，≤2000 条，超出标
  `truncated`）、`expansions`（扩大项，界面标红）、`narrowings`、`base_effective_hash`、`current_version`、`body_sha256`。
- **扩大项判定**：能力集——白名单新增或变宽、规则名单新增（`read.rules_whitelist`：新类型记「新类型首次获得规则读取」，
  已有类型记「规则读取范围变宽」；移除项记收窄「规则读取范围收窄」）、`content_patch_only`／`propose_desired` false→true、硬绑定被解除、首次授予；
  skills——被硬绑定的 skill 正文变更（标「人工判断」）；rules——被删除的含「禁止／绝不／不得／一律／必须／永不／必叠」的行
  （候选，人工判断）；撤销整份母本恒为扩大项。
- **发布**（`POST /api/v1/policy/activate`，L3）：请求三选一 `{desired_id}` / `{rollback_to}` / `{release_global_revocation:true}`，
  必带 `expected_effective_hash`（CAS）。服务端固定顺序：重验会话 → 审计 **Intent**（`policy.activated`）→
  `require_l3(ActivatePolicy)` → 策略库事务（哈希 CAS → 重新校验 → 写版本 → 更新现行 → 重算 effective 哈希 →
  取代同 subject 其余 desired → 能力集发布顺带解除该类型吊销）→ 审计 **Outcome**（同 id）。
  拒绝一律 `policy.activate_denied{reason}`：`reauthentication_required`（403）、`no_l3_path`（409）、`untrusted_source`（403）、
  `hash_mismatch`（409，回传 `current_effective_hash`）、`validation_failed`（422）、`not_found`（404）、`not_activatable`（409）。
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
- `propose_desired{kind,subject,body}`：需能力集 `policy.propose_desired`；入库 `pending`，记 `policy.desired_submitted`；
  返回 id／状态／扩大项计数／逐条问题；**不改变 effective**。

## 6. 错误模式

| 错误 | 触发 | 对外 |
|---|---|---|
| `InvalidKind` / `InvalidSubject` / `InvalidBody` | 类别、subject 或正文不合规 | 400；入库前拒绝，不留记录 |
| `ValidationFailed` | 发布时重新校验未通过 | 422 ＋ desired 报告更新 |
| `HashMismatch{current}` | CAS 失败 | 409 ＋ `current_effective_hash` |
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
- `crates/agent-server/tests/rules_access_contract.rs`：规则名单对 `policy_read` 清单与单条、握手路牌、`read_note`／
  `list_entries`／`search` 同一裁决，名单外与不存在（或白名单外）对外同一错误、审计 `rule_not_whitelisted`，
  `_规则/` 子目录照项目白名单，缺字段零规则，M3 兼容模式不过滤；`crates/supply/tests/rules_files_contract.rs`：
  规则文件判定与路牌库文件兜底按名单裁决。
- `crates/agent-server/tests/policy_contract.rs`：零权限握手、供给视图求交、路牌 3／5／6 项、`policy_read` 审计与索引、
  `propose_desired` 永不生效与能力否决、三级吊销即刻生效与恢复、HTTP 外壳同一授权、M3 兼容模式。
- `crates/local-server/tests/policy_http_contract.rs`：真实 Unix 入口＋TOTP：发布挡在 L3 后（用例 36 人侧）、Intent/Outcome
  同 id、CAS 409、回滚、版本读取、三级吊销 L2 与全局解除 L3、方法与查询封闭、未装配策略平面 404。
- 前端 `web/src/test/policy*`：模型封闭解析、扩大项标红、吊销提交、未接入显示；发布遇 403 `reauthentication_required` 就地弹二次认证窗口，认证通过后**按原 `expected_effective_hash`** 自动重试同一次动作（取消或认证失败都不重试；期间 effective 变了仍按原 hash 提交、由服务端 `hash_mismatch` 挡下；本入口无 One-time Code 路径时窗口直说不能发布）。
