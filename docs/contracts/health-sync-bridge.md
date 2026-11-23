# 同步健康与人工账号配额契约（`GET /api/v1/health`）

T-132 P1（ADR-025 D11），2026-09-23；T-205 于 2026-11-13 补齐只读界面。本文记录同步状态、
`sync-transport` 活性回执、镜像范围配额和人工账号配额。
入口、鉴权与整体三态规则不变，见 [health-and-alerting](../m2/health-and-alerting.md) §2、§8、§10。
实现位置：

- 判定：`crates/observability/src/health/bridge.rs`（`BridgeEvidence`／`bridge_section`），聚合在 `aggregate.rs`。
- 证据来源：`crates/local-server/src/sync_plane/health.rs`（`PlaneHealth`，实现 `BridgeHealthProbe`），
  组合根接线 `crates/local-server/src/startup/health_input.rs`。
- 前端解码：`web/src/healthApi.ts`、`web/src/health/model.ts`；结构化明细在 `health/BridgeDetails.tsx`。
  驾驶舱「双向同步」格读取 `sync-bridge` 状态，不从文案提取秒数。

## 1. 响应形状

顶层为 `{overall, evidence_complete, sections, bridge, manual_sync_quota}`。生产健康页使用同一次
`EvidenceSnapshot`，七项检查中新增 `manual-sync-quota`。`bridge` 未装配时为 `null`，否则为下面的
封闭对象；每项未取得的证据单独为 `null`。已知故障时仍展示其他已知明细。

| 字段 | 形状 |
|---|---|
| `last_batch` | `null` 或 `{sequence, closed_at}`；序号为 u64 十进制字符串，时刻为 UTC RFC3339 或 `null` |
| `fused` / `quiescence_valid` / `config_matches` | 布尔或 `null` |
| `unknown_jobs` / `conflict_copies` / `mirror_bytes` / `deletes_blocked` | u64 十进制字符串或 `null`，避免浏览器整数精度丢失 |
| `last_exit` | `clean` / `non_zero` / `signal` / `deadline_exceeded` / `owner_stopped` / `unconfirmed` / `null` |
| `recovery` | `idle` / `forced_mirror_remote` / `backoff` / `null` |
| `residue_guard` | `clear` / `blocked` / `null` |
| `mode` | `mirror-remote` / `bidirectional` / `null` |

`sections` 每项仍是 `{code, title, state, detail}`。`state` 只有 `normal`／`degraded`／`unknown`，
整体取最重一项，全部证据已知才可标 `evidence_complete`。`detail` 的构造期脱敏规则不变。
明细不包含路径、凭证、远端地址、主机名、stderr 或身份。

前端保留旧三字段健康响应的兼容读取；出现结构化字段时严格校验闭集、u64、时间和结论一致性。
重复检查项、缺字段、多余字段、未知枚举或与明细矛盾的正常结论均不能显示正常；已知降级项仍保留。

## 2. 第六节 `sync-bridge`

| 字段 | 固定值 |
|---|---|
| `code` | `sync-bridge` |
| `title` | `双向 Bridge` |

判定顺序固定（`bridge.rs::judge`），取第一条命中的：

| # | 条件 | `state` | `detail` 模板 |
|---|---|---|---|
| 0 | 平面未装配（默认关闭）或证据读不到 | `unknown` | `驱动平面未装配或证据不可读，无法判断双向同步状态` |
| 0′ | 最近一次走到删除守卫的那一轮被挡（`deletes_blocked` > 0，BUG-046），且删除熔断没有证实生效（`fused` 不是 `true`）；有正证据就报，不等别的字段齐 | `degraded` | `远端删了 <n> 个文件（改名也算），主库没开文件操作，同步整批暂停：到远端恢复这些文件或开文件操作后接着跑` |
| 1 | 十一个字段任一缺失 | `unknown` | `Bridge 证据不完整：缺<字段中文名>`，已知上次非干净退出时附带该退出分类 |
| 2 | 删除熔断生效 | `degraded` | `删除熔断生效，本批与后续批次全部阻断` |
| 3 | 排除项守卫阻断 | `degraded` | `排除项清除前有镜像残留` |
| 4 | headless 配置与审批不一致（模式／排除项／文件类型） | `degraded` | `headless 配置与审批文档不一致，本轮未同步` |
| 5 | 有终态未知的动作 | `degraded` | `有 <n> 条动作终态未知，等待原键核对` |
| 6 | 上次 headless 非干净退出 | `degraded` | `上次 headless 运行<退出分类>` |
| 7 | 恢复协议进行中 | `degraded` | `恢复协议进行中：<协议状态>` |
| 8 | 静止证据过期 | `unknown` | `静止证据已过期，等待下一轮重新取得` |
| 9 | 以上都不是 | `normal` | `第 <seq> 批于 <unix> 闭合；冲突副本 <n>；镜像 <bytes> 字节（非账号用量）；模式 <mode>` |

字段中文名（第 1 条）：最近一批、删除熔断、终态未知动作数、静止证据、冲突副本数、镜像字节数、退出分类、
恢复协议状态、排除项守卫、镜像模式、配置核对。`deletes_blocked` 不在其中：还没有一轮证明过（`null`）不算缺字段，前端「能否正常」用同一口径（`null` 或 `0` 才可正常）。第 0′ 条排在缺字段之前，是因为被挡的那几轮不走执行段，重启后「最近一批」等字段一直补不齐，照旧顺序只会显示「证据不完整」；熔断已证实生效时让给第 2 条：熔断持久、没有解除入口，开了文件操作也照样挡，是更该先处理的那一个。退出分类（第 6 条）：以非零退出码结束／被信号终止／超过时限被停止／
在 owner 关停时被停止／退出结果未确认。协议状态（第 7 条）：强制只下载恢复一次／退避等待。第 9 条的
`冲突副本 <n>` ＝ 最近一次完整观察里两侧命名为冲突副本的路径数（headless 与 studiolo-canonical 两种设备名都识别）
＋ 持久 `conflict_stamps` 的当前条数（启动、配戳与回收后覆盖回填；包含已配戳、正在续写副本的冲突，
不代表 `ops/` 的历史副本文件总数）。模式：`mirror-remote`／
`bidirectional`，与审批文档 `headless_mode` 同一字面量。`<unix>` 是该批闭合的 Unix 秒整数，前端要显示时刻自行格式化。

前端要接的字段：`code` 用于定位与着色分支，`title` 直接显示，`state` 决定颜色，`detail` 整句显示；
不要解析 `detail` 里的数字做二次判断——数字只供人读，判定以 `state` 为准。

### 平面装配后各字段何时有值

`PlaneHealth` 只写驱动器证明过的字段，没证明过的保持缺失（落第 1 条，未知）：

| 字段 | 来源 |
|---|---|
| 最近一批 | 轮末一批闭合或停住（`record_round`） |
| 删除熔断、终态未知动作数、冲突副本数 | 执行段回填 `record_execution`（冲突副本数＝`record_observed_copies` 的观察数＋`record_conflicts` 的持久冲突数）；一批闭合的轮末把前两项复位为未生效／0 |
| 静止证据 | 轮末置位：除退避跳过外的成功轮为有效，任何失败轮为过期 |
| 镜像字节数 | 观察镜像后 `record_usage` |
| 退出分类、恢复协议状态 | 启动时从传输存储 `last_terminal` 恢复；headless 回收后 `record_headless`（按退出码、停止原因与静止证据分类），非干净闭合报 `Headless`、轮末保留分类；真正的传输错误报 `Transport` 并置未确认；跑到执行段的轮末也复位为干净退出／空闲 |
| 排除项守卫 | 守卫阻断的轮末为 `Blocked`；跑到执行段的轮末为 `Clear` |
| 镜像模式 | InspectConfig 复核通过后 `record_mode` |
| 删除被挡 | 每一轮走到第 9′ 步删除守卫都由守卫覆盖写（`record_delete_gate`）：被挡记 n，放行记 0；续做旧批次的那一轮也照报被挡的个数（旧批次一闭合，下一轮就整批暂停）；没走到守卫的轮次（退避、强制恢复、上一批仍有未知终态、守卫之前失败）不动它。轮末结论不写它；`DeletesBlocked{n}` 那一轮 RunOnce 已干净回收，轮末同样复位退出分类、恢复协议状态与排除项守卫，但不盖最近一批 |

因此平面刚装配、第一批还没闭合时，本节是 `unknown`（缺最近一批），不是 `normal`；启动恢复的非干净退出（含 owner 关停）会附在缺项提示后，避免已知事实被遮住；
一轮失败后即使上一批曾正常，本节也至少回到 `unknown`（静止证据过期），不会停留在绿。

## 3. `owner-liveness` 新增 `sync-transport`

平面装配后 `owner-liveness` 的证据里多一个 owner，标签 `sync-transport`：回执时刻＝最近一轮结束时刻
（无论那轮成功与否），约定周期 600 秒（两倍轮间隔）。一轮没跑过时它是"证据不足"，超过 600 秒没有新一轮即"超期"。
`detail` 仍是既有模板 `活性回执正常 <n> 个，超期 <n> 个，证据不足 <n> 个`；备份 Worker 仍未接线，
所以平面装配后这一节的 owner 只有它一个，没装配时仍是 `没有任何 owner 活性证据`。

## 4. `sync-quota` 的正文后缀

`sync-quota` 的 `detail` 固定以 `（非账号用量）` 结尾，四种结论都带：

| `state` | `detail` |
|---|---|
| `normal` | `占用在预警水位以内（非账号用量）` |
| `degraded` | `占用已越过预警水位，尚未越过上限（非账号用量）` |
| `degraded` | `占用已越过上限，对应写入路径已阻断（非账号用量）` |
| `unknown` | `占用读数不可得，无法判断（非账号用量）` |

读数是 Studiolo 侧镜像的字节数与对象数，**不是** Obsidian 账号用量——机器读不到账号用量。上限是审批文档里
人工抄录的 `manual_quota.account_limit_bytes`，预警取七成；对象维度取 `SyncScope` 固定的 4096 与其七成（2867）。
审批文档没有 `manual_quota`、平面未装配、或本轮还没观察过镜像，都是 `unknown`。`disk-quota` 不带后缀，正文不变。

## 5. 契约测试

- `crates/observability/tests/health_contract.rs` 与 `health_contract/bridge.rs`：判定顺序、每个字段缺失、
  三组标签、配额后缀、公开摘要。
- `crates/local-server/src/health/tests/probe.rs`、`src/sync_plane/health/tests.rs`、
  `src/startup/health_input.rs`（`tests`）：探针接线、回填语义、七成预警。

## 6. 人工账号配额（T-205）

`sync-quota` 标题改为「镜像范围配额」，仍沿用 §4 的读数与准入规则。新增 `manual-sync-quota`
仅判断配置中的人工记录，不改变写入准入，不主动读取或修改 Obsidian 账号。

`manual_sync_quota` 未配置时为 `null`；配置后固定八字段：`account_limit_bytes`、`used_bytes_manual`
（u64 十进制字符串）、`recorded_at`、`as_of`（UTC RFC3339 或 `null`）、`warning_percent: 70`、
`max_age_seconds: 86400`、`state`、`reason`。两项时刻分别是人工记录时刻与本次快照时刻。

判定按顺序：上限 0 → `unknown/invalid`；时刻无效、未来或差值溢出 → `unknown/clock`；
记录超过 24 小时 → `unknown/stale`；记录用量大于上限 → `degraded/exceeded`；
达到 70%（含等号）→ `degraded/warning`；其他 → `normal/within`。缺记录的检查项为未知。
乘法使用 u128，浏览器复核使用 BigInt；不丢失 u64 最大值。

健康浮层展示人工用量、上限、比例、记录时间及更新提示，并明确「实时账号用量未知」。
镜像字节另列且标「非账号用量」。明细触点最小 44px，小于 600px 改单列。

新增契约：`local-server/src/health/tests/bridge_details.rs` 与 `web/src/test/healthBridge.test.tsx`，
覆盖七成边界、过期／未来时钟、整数上限、闭集及矛盾结论；启动、原健康和驾驶舱契约同步更新。
