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
| `last_batch` | `null` 或 `{sequence, closed_at}`：最近**闭合**的那一批（BUG-048）——一批闭合（`Settled`）那一轮结束时盖上序号和这一轮结束的时刻；停住（终态未知、冻结）的轮次不动它，本进程还没有一批闭合过就是 `null`。序号为 u64 十进制字符串，时刻为 UTC RFC3339 或 `null`（转不出 RFC3339 时） |
| `fused` / `quiescence_valid` / `config_matches` | 布尔或 `null` |
| `unknown_jobs` / `conflict_copies` / `mirror_bytes` / `deletes_blocked` | u64 十进制字符串或 `null`，避免浏览器整数精度丢失 |
| `last_exit` | `clean` / `non_zero` / `signal` / `deadline_exceeded` / `owner_stopped` / `unconfirmed` / `null` |
| `recovery` | `idle` / `forced_mirror_remote` / `backoff` / `null` |
| `residue_guard` | `clear` / `blocked` / `null` |
| `mode` | `mirror-remote` / `bidirectional` / `null` |
| `path_usage` | `null` 或 `{used, budget}`：最近一次算出的三方并集路径数与同步路径预算，两个都是 u64 十进制字符串（BUG-047） |
| `byte_usage` | `null` 或 `{used, budget}`：最近一次走到批准之前算出的比对内容字节数（基线、canonical、镜像三方「存在」内容各算一遍的合计）与字节预算 4 GiB，两个都是 u64 十进制字符串 |

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
| 1 | 有任一条正证据（下表），不等别的字段齐 | `degraded` | `<排第一的那条的整句>`，还有别的正证据时接 `；另有：<其余短名，按下表顺序用「、」隔开>`，十一个字段还缺时再接 `；证据不完整：缺<第一个缺的字段中文名>` |
| 2 | 十一个字段任一缺失 | `unknown` | `Bridge 证据不完整：缺<字段中文名>` |
| 3 | 静止证据过期 | `unknown` | `静止证据已过期，等待下一轮重新取得` |
| 4 | 以上都不是 | `normal` | `第 <seq> 批于 <unix> 闭合；冲突副本 <n>；镜像 <bytes> 字节（非账号用量）；路径 <used>／<budget>；比对内容 <p>%；模式 <mode>`（还没算到过路径用量时没有「；路径 …」这一段，还没算到过比对内容时没有「；比对内容 …」这一段；比对内容只写百分比，确切字节数在 `byte_usage` 里） |

正证据（第 1 条）指已经证实的问题，按固定顺序排：

| 顺序 | 条件 | 排第一时的整句 | 短名 |
|---|---|---|---|
| ⓪ | 三方并集超过同步路径预算（`path_usage.used` > `budget`，BUG-047） | `这一轮要比对 <used> 条路径，超过上限 <budget> 条，同步停住：要出新版提高上限` | 路径超上限 |
| ① | 三方比对内容超过字节预算（`byte_usage.used` > `budget`） | `这一轮要比对的内容合计 <used> 字节，超过上限 <budget> 字节，同步停住：要出新版提高上限，或移走范围里的大文件` | 比对内容超上限 |
| ② | 删除熔断生效（`fused` 为 `true`） | `删除熔断生效，本批与后续批次全部阻断` | 删除熔断 |
| ③ | 最近一次走到删除守卫的那一轮被挡（`deletes_blocked` > 0，BUG-046） | `远端删了 <n> 个文件（改名也算），主库没开文件操作，同步整批暂停：到远端恢复这些文件或开文件操作后接着跑` | 删除被挡 |
| ④ | 排除项守卫阻断（`residue_guard` 为 `blocked`） | `排除项清除前有镜像残留` | 排除项残留 |
| ⑤ | headless 配置与审批不一致（`config_matches` 为 `false`：模式／排除项／文件类型） | `headless 配置与审批文档不一致，本轮未同步` | 配置不符 |
| ⑥ | 有终态未知的动作（`unknown_jobs` > 0） | `有 <n> 条动作终态未知，等待原键核对` | 终态未知动作 |
| ⑦ | 上次 headless 非干净退出（`last_exit` 已知且不是 `clean`） | `上次 headless 运行<退出分类>` | 非干净退出 |
| ⑧ | 恢复协议进行中（`recovery` 已知且不是 `idle`） | `恢复协议进行中：<协议状态>` | 恢复协议进行中 |
| ⑨ | 路径用量到预算的八成（`used × 5 ≥ budget × 4`，含恰好用满；u128 算） | `路径用量 <used>／<budget>，已到上限的 <p>%：该安排提高上限`（`p` 向下取整） | 路径接近上限 |
| ⑩ | 比对内容到字节预算的八成（同一算法） | `比对内容 <used>／<budget> 字节，已到上限的 <p>%：该安排提高上限，或移走范围里的大文件` | 比对内容接近上限 |

⓪ 永远排第一：超上限时整轮观察停住，别的字段都不会再更新，提高上限之前其余问题都处理不了；
① 紧随其后：这一轮停在批准之前，同样走不到后面的步骤（路径那道检查更早，两条都在时先报路径）。比对内容把同一份文件在基线、
canonical、镜像各算一遍，范围里的内容到上限的三分之一左右（约 1.33 GiB）就会顶到；处理是出新版提高上限，或移走范围里的大文件
（主库没开文件操作时，远端的删除会落到 ③ 删除被挡，要等文件操作开好才走得通）。
⑨、⑩ 只是提前提醒，排最后。镜像那侧走到文件数上限（镜像自己就有 16384 个文件）时只能判观察不完整、说不出确切条数，那之前 ⑨ 早就亮了。

顺序不看别的字段齐不齐，没有例外。例：②～⑧ 都在时正文是
`删除熔断生效，本批与后续批次全部阻断；另有：删除被挡、排除项残留、配置不符、终态未知动作、非干净退出、恢复协议进行中`（字段还缺再接缺的字段）；
重启后只知道上次 headless 在 owner 关停时被停下、恢复协议要强制只下载一次时是
`上次 headless 运行在 owner 关停时被停止；另有：恢复协议进行中；证据不完整：缺最近一批`。

字段中文名（第 1、2 条）：最近一批、删除熔断、终态未知动作数、静止证据、冲突副本数、镜像字节数、退出分类、
恢复协议状态、排除项守卫、镜像模式、配置核对，按这个顺序取第一个缺的。`deletes_blocked`、`path_usage`、`byte_usage` 不在其中：还没有一轮证明过（`null`）不算缺字段，前端「能否正常」用同一口径（被挡数 `null` 或 `0`，路径用量、比对内容用量各自 `null` 或不到八成才可正常）。
正证据排在缺字段之前（T-223），是因为人侧重启后「最近一批」等字段要等执行段跑过一批才补得齐，删除被挡的那几轮根本不走执行段；
照缺字段优先的旧顺序，这段时间只会显示「证据不完整」，已知的原因被盖住（BUG-046 停摆 51 天期间健康页很可能就是这样）。
已知的值不是问题（未熔断、0 条未知、守卫放行、配置一致、干净退出、协议空闲、被挡 0 个）不算正证据，字段不齐时照旧落第 2 条。
「另有」只列短名，数目和分类看结构化明细（`bridge` 对象）；静止证据过期只是「未知」级别，不进「另有」。
熔断排在删除被挡前面（T-222）：熔断持久、没有解除入口，开了文件操作也照样挡，是更该先处理的那一个。T-222 原来还要求十一个字段都齐，
因为字段不齐时熔断报不出来；T-223 之后字段不齐也报得出来，用户 2026-11-22 定去掉这个条件，重启后熔断冻结着旧批次、续做那一轮守卫仍挡时也先报熔断，被挡列在「另有」里。
正文上限 200 字（`HealthSection::new` 越界即整页聚合失败）：降级正文最长的是 ① 排第一、②～⑨ 都在、计数取 u64 上限、还缺字段，154 字；
正常正文每个数都取最长时 180 字（所以比对内容只写百分比：再带两个字节数就超了），都有用例钉住。
退出分类（⑦）：以非零退出码结束／被信号终止／超过时限被停止／
在 owner 关停时被停止／退出结果未确认。协议状态（⑧）：强制只下载恢复一次／退避等待。第 4 条的
`冲突副本 <n>` ＝ 最近一次完整观察里两侧命名为冲突副本的路径数（headless 与 studiolo-canonical 两种设备名都识别）
＋ 持久 `conflict_stamps` 的当前条数（启动、配戳与回收后覆盖回填；包含已配戳、正在续写副本的冲突，
不代表 `ops/` 的历史副本文件总数）。模式：`mirror-remote`／
`bidirectional`，与审批文档 `headless_mode` 同一字面量。`<unix>` 是该批闭合那一轮结束的 Unix 秒整数（闭合在轮内完成，
这时它一定已经闭合；停住的批次不盖，见下表「最近一批」），前端要显示时刻自行格式化。

前端要接的字段：`code` 用于定位与着色分支，`title` 直接显示，`state` 决定颜色，`detail` 整句显示；
不要解析 `detail` 里的数字做二次判断——数字只供人读，判定以 `state` 为准。

### 平面装配后各字段何时有值

`PlaneHealth` 只写驱动器证明过的字段，没证明过的保持缺失（没有正证据时落第 2 条，未知）：

| 字段 | 来源 |
|---|---|
| 最近一批 | 轮末一批闭合（`Settled`）时由 `record_round` 盖上序号与这一轮结束的时刻；终态未知（`Unknown`）、冻结（`Blocked`）的轮次不动它（BUG-048：停住的轮次原来也盖，卡住几十天时刻照样每轮往前走，读的人会以为同步在正常闭合）。卡住的批次看终态未知动作数、删除熔断两项 |
| 删除熔断、终态未知动作数、冲突副本数 | 执行段回填 `record_execution`（冲突副本数＝`record_observed_copies` 的观察数＋`record_conflicts` 的持久冲突数）；一批闭合的轮末把前两项复位为未生效／0 |
| 静止证据 | 轮末置位：除退避跳过外的成功轮为有效，任何失败轮为过期 |
| 镜像字节数 | 观察镜像后 `record_usage` |
| 退出分类、恢复协议状态 | 启动时从传输存储 `last_terminal` 恢复；headless 回收后 `record_headless`（按退出码、停止原因与静止证据分类），非干净闭合报 `Headless`、轮末保留分类；真正的传输错误报 `Transport` 并置未确认；跑到执行段的轮末也复位为干净退出／空闲 |
| 排除项守卫 | 守卫阻断的轮末为 `Blocked`；跑到执行段的轮末为 `Clear` |
| 镜像模式 | InspectConfig 复核通过后 `record_mode` |
| 路径用量 | 每次观察算出三方并集（基线 ∪ canonical ∪ 镜像）、第二次 canonical 观察之前由 `witness::budget::check` 覆盖写（`record_path_usage`）；超上限也照实记，这一轮按观察失败收工，并打一行 `studiolo_bridge step=Observe outcome=path_budget_exceeded used=<n> budget=<N>`；没走到这一步的轮次不动它，轮末结论不写它 |
| 比对内容用量 | 两侧观察都拿到之后、交 `SyncScope::approve_union` 之前，由 `witness::budget::check_bytes` 覆盖写（`record_byte_usage`）：基线、canonical、镜像三组证据里每个「存在」条目的字节数相加（同一份文件三方都有就算三遍，和范围并集的累计口径相同，溢出按饱和算）；超上限也照实记，这一轮按观察失败收工，并打一行 `studiolo_bridge step=Approve outcome=byte_budget_exceeded used=<n> budget=<N>`；范围并集里同一道检查留作兜底。没走到批准的轮次不动它，轮末结论不写它 |
| 删除被挡 | 每一轮走到第 9′ 步删除守卫都由守卫覆盖写（`record_delete_gate`）：被挡记 n，放行记 0；续做旧批次的那一轮也照报被挡的个数（旧批次一闭合，下一轮就整批暂停）；没走到守卫的轮次（退避、强制恢复、上一批仍有未知终态、守卫之前失败）不动它。轮末结论不写它；`DeletesBlocked{n}` 那一轮 RunOnce 已干净回收，轮末同样复位退出分类、恢复协议状态与排除项守卫，但不盖最近一批 |

因此平面刚装配、第一批还没闭合、也没有正证据时，本节是 `unknown`（缺最近一批），不是 `normal`；
启动时从 `last_terminal` 恢复出的非干净退出（含 owner 关停）和恢复协议进行中都是正证据，本节判 `degraded`、先报它们再点名缺的字段，已知事实不被遮住；
删除被挡之后一直走不到守卫的轮次（退避、强制恢复、上一批仍有未知终态、RunOnce 连续非干净退出）沿用上次证明过的个数，正文仍以它开头，
个数可能是旧的（契约说的就是「最近一次走到守卫的那一轮」），这期间新证实的原因列在「另有」里，不被它盖住；
一轮失败后即使上一批曾正常，本节也至少回到 `unknown`（静止证据过期），不会停留在绿。

## 3. `owner-liveness` 新增 `sync-transport`

平面装配后 `owner-liveness` 的证据里多一个 owner，标签 `sync-transport`：回执时刻＝最近一轮结束时刻
（无论那轮成功与否），约定周期 600 秒（两倍轮间隔）。结束时刻是驱动器返回之后另取的（`sync_plane::run_and_record`），
不是喂给这一轮的开始时刻（BUG-049：原来记的是开始时刻，回执年龄多算上一轮的用时）；同步状态回执里「对上」「守卫证明」
照旧取开始时刻，见 [sync-status-receipt](sync-status-receipt.md)。一轮没跑过时它是"证据不足"，超过 600 秒没有新一轮即"超期"。
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
人工抄录的 `manual_quota.account_limit_bytes`，预警取七成；对象维度取同步路径预算 16384（`studiolo_core::SYNC_PATH_BUDGET`，BUG-047 之前是 4096）与其七成（11468）。
审批文档没有 `manual_quota`、平面未装配、或本轮还没观察过镜像，都是 `unknown`。`disk-quota` 不带后缀，正文不变。

## 5. 契约测试

- `crates/observability/tests/health_contract.rs` 与 `health_contract/bridge.rs`：判定顺序、每个字段缺失、
  三组标签、配额后缀、公开摘要。
- `crates/local-server/src/health/tests/probe.rs`、`src/sync_plane/health/tests.rs`、
  `src/startup/health_input.rs`（`tests`）：探针接线、回填语义、七成预警。
- `crates/local-server/tests/sync_plane_health/round_times.rs`（挂在 `sync_plane/health/tests.rs`）：一轮的两个时刻各落到哪
  （活性回执、一批闭合取结束，对上、守卫证明取开始），停住的轮次不让最近一批前移（BUG-048、BUG-049）。

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
