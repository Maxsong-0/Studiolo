# 健康页、配额、白名单诊断、带外告警与死人开关（T-048 · B2）

本文记录 M0–M3 收尾 B2 段的实施结果。权威来源：
`docs/m2/m2-m3-closeout.md` 台账「备份恢复和运维」行，以及 Vault
`04-协调/2026-09-17-M0-M3收尾计划.md` 的 B2 段。

## 0. 秘密纪律（最重要的一条）

**收件地址、SMTP 凭据、任何 token 都不在本仓库、不在本文档、不在测试夹具里。**
真实发送端只经两个环境变量注入：

| 环境变量 | 含义 |
|---|---|
| `STUDIOLO_ALERT_SMTP_URL` | SMTP 端点（含凭据）的完整 URL |
| `STUDIOLO_ALERT_TO` | 带外告警收件地址 |

值去哪拿：本机私有 `owner-inputs.json`（不进仓库、不进 Vault、不经聊天）。
任一变量缺失或为空，transport 一律返回「未配置」，**不会静默当作发送成功**。
主人审批文档 (`studiolo-host-config::alerting`) 用 `deny_unknown_fields`，
任何形似 `alert_to` / `smtp_url` 的字段都会被直接判非法，杜绝"顺手写进配置"。

## 1. 模块分布

| 模块 | 职责 | IO |
|---|---|---|
| `crates/observability/src/health/{model,aggregate,diagnostics,bridge}.rs` | 三态模型、只读聚合、白名单一致性诊断、双向 Bridge 判定 | 无 |
| `crates/local-server/src/sync_plane/health.rs` | 双向 Bridge 平面的健康累积器（`BridgeHealthProbe`，T-132 P1） | 无 |
| `crates/observability/src/quota/{disk,sync}.rs` | 磁盘与 Sync 配额判定与写入准入 | 无 |
| `crates/observability/src/alert/{transport,dispatch,deadman}.rs` | 通道抽象、审计＋幂等＋有界重试、死人开关状态机 | 无 |
| `crates/observability/src/store/state.rs` | 持久文档编解码与校验和 | 无 |
| `crates/observability/src/store/storage.rs` | **本 crate 唯一真实 IO**：固定 `deadman-state.json` | 有 |
| `crates/host-config/src/alerting/**` | 主人审批文档（纯解析，只有数字与角色标签） | 无 |
| `crates/local-server/src/health/**` | `/api/v1/health` 只读入口与响应塑形 | 无 |
| `web/src/healthApi.ts`、`web/src/Health*.tsx` | 前端只读健康页 | 无 |

## 2. 健康页：本地只读聚合

六节固定检查项，机器码稳定：

| 机器码 | 中文标题 | 判定 |
|---|---|---|
| `recovery-point` | 恢复点年龄 | 按**内容截止时刻**算年龄，不按上传完成时刻 |
| `owner-liveness` | 各 owner 活性 | 逐个组件角色比对回执与约定周期 |
| `disk-quota` | 磁盘配额 | 见 §3 |
| `sync-quota` | Sync 配额 | 见 §3；正文固定带「非账号用量」 |
| `whitelist-diagnosis` | 白名单诊断 | 见 §4 |
| `sync-bridge` | 双向 Bridge | T-132 P1 契约 `docs/contracts/health-sync-bridge.md`；平面未装配即未知 |

三态与汇总规则：`Degraded`（2）＞ `Unknown`（1）＞ `Normal`（0），
整体取最重的一节。于是——

- 只要有一节缺证据，整体最少是「未知」，**不可能显示绿**；
- 已知故障不会被「未知」掩盖；
- 检查项为空也判「未知」。

首次 seed 校验前，恢复点一节固定显示 **「尚未建立灾难恢复点」** 且状态为未知。
本地时钟早于恢复点内容截止时刻（或差值越界）时显示「时间证据不可信」，仍然不是绿。

**防泄露是构造期约束，不是自觉**：`HealthSection::new` 与 `AlertMessage::new`
拒绝含 `/`、`\`、`@`、控制字符或超过 200 字符的正文；
`HealthReport::public_summary()` 的结构里只有机器码与状态，装不下路径或身份。

## 3. 配额：显式上限 ＋ 显式预警水位 ＋ 阻断写入

两条维度各自独立：

- 磁盘：`minimum_free_bytes`（阻断下限）与 `warn_free_bytes`（预警水位）。
  准入规则：写完之后剩余空间仍须 **≥ 下限，等号可过**。
- Sync：范围内 `max_objects` / `max_bytes` 与对应预警水位。
  准入规则：加上本次增量后 **≤ 上限，等号可过**。

纪律：

- 超限**阻断对应写入路径**（`Admission::Blocked`），不是只告警；
- 所有整数走 `checked_add` / `checked_sub`，溢出返回 `QuotaError::ArithmeticOverflow`，
  一律拒绝，**绝不回绕后放行**；
- **读数缺失不得放行**：没有证据就按不满足下限处理，判定侧同时判「未知」。

## 4. 白名单诊断：只读，不自行修配置

输入是一份纯快照 DTO（`WhitelistSnapshot`），由装配层把已加载的
`SyncScope` / `file_operations` / `human_content` 生成区摊平后传入——
`observability` 不依赖 `host-config`，因此诊断不可能反过来改配置。

冲突分类（稳定机器码）：

| 机器码 | 含义 |
|---|---|
| `generated-region-is-writable` | 生成区同时落在人侧可写范围内 |
| `generated-region-inside-sync-scope` | 生成区同时落在双向搬运范围内 |
| `sync-scope-covers-protected` | 搬运范围覆盖了保护前缀 |
| `writable-covers-protected` | 人侧可写范围覆盖了保护前缀 |
| `human-folder-inside-protected` | 人写目录整体落在保护前缀之内 |
| `duplicate-registration` | 同一张登记表内重复登记 |
| `whole-file-shadowed-by-folder` | 整文件登记被同范围整目录规则遮蔽 |

覆盖判定只在**路径分段边界**上成立（`Share` 不覆盖 `Shared`）。
登记总数超过 4096 条即 `TooManyEntries` **失败关闭**，健康页显示「未知」，
绝不因为跑不动就报告「一致」。

## 5. 带外告警

`AlertTransport` 是 trait：

```rust
fn deliver(&self, message: &AlertMessage) -> Result<DeliveryReceipt, TransportError>;
```

失败分类固定且可核对：`not-configured` / `timeout` / `unavailable` /
`rejected` / `malformed`。只有 `timeout` 与 `unavailable` 可重试。
分类串不含任何 token 片段；`EnvironmentSmtpTransport` 的 `Debug`
只输出 `configured: bool`，连变量名和值都不出现。

**当前仓库没有装配真实 SMTP 客户端。** 即使两个环境变量齐备，
`EnvironmentSmtpTransport::deliver` 也返回 `unavailable` 而**不是**伪装送达。
这是「不冒称未验证的能力」：发送端接上之后只需替换 `deliver` 的实现，
配置读取路径与调度语义都不用改。

调度顺序是硬约束：

1. 已送达的幂等键直接短路（`AlreadyDelivered`），不重复打扰；
2. 写 **Intent** 审计，动作固定 `observability.alert.dispatch`；
   **写不进去就一次都不发**，返回 `AuditUnavailable`；
3. 调 transport，可重试错误按 `max_attempts` 有界重试（无"无限重试"）；
4. 写 **Outcome** 审计，附固定分类与实际尝试次数。

审计 payload 只放 `alert_code` / `severity` / `idempotency_key` /
`classification` / `attempts`，**不放告警正文、收件地址、凭据或原配置**。
审计事件 id 形如 `alert-<幂等键>-<调度轮次>`：同一轮内的 transport 重试共享
一条事件，不同轮各自留痕，因此重复调度不会撞上幂等冲突。

> **待接线**：固定动作 `observability.alert.dispatch` 需要登记进
> Vault `01-框架/敏感动作枚举表.md`（root 统一处理）。

## 6. 死人开关

参数：`period_seconds`（约定回执周期）、`debounce_seconds`（去抖）、
`cooldown_seconds`（冷却）、`max_suppressed`（冷却期压制上限，必须 > 0）。

状态迁移（`alert/deadman.rs`，纯函数）：

1. **时钟回退优先**：`now < last_observed` ⇒ 报「时间证据不可信」，
   开新情节并**立即触发**，不受冷却压制，**绝不静默跳过**；
   时间差越界同样按此处理。
2. 回执距今 ≤ 周期 ⇒ 正常，清空压制计数。
3. 超期但未越过去抖窗口 ⇒ 本轮不报（`Debouncing`）。
4. 越过去抖窗口：
   - 不在冷却期 ⇒ 触发 `no-receipt`；
   - 在冷却期且压制次数 < 上限 ⇒ 压制，但**次数被记进持久状态**；
   - 在冷却期且压制次数已达上限 ⇒ **强制升级**触发
     `escalated-during-cooldown`（新情节、新幂等键）。
     这条保证冷却**不会把连续故障压成一次都不报**。
5. 从未收到回执时以状态起点为锚，不当作"刚刚收到"。

幂等键 = `sha256("studiolo.alert.idempotency.v1" ‖ "deadman-switch" ‖ reason ‖ episode)`
的前 16 字节十六进制：同一情节跨进程、跨重启稳定，冷却结束后复用同一把键，
由调度器去重；升级则换新情节因而换新键，保证"再报一次"确实报得出来。

## 7. 持久状态：`deadman-state.json`

固定私有目录下只碰两个固定名字：`deadman-state.json` 与暂存件
`deadman-state.json.new`。

写：先清掉上一轮崩在中途留下的暂存件 → `O_CREAT|O_EXCL|O_NOFOLLOW` 独占创建
→ 写满 → `fsync` → `renameat` 完整替换 → `fsync` 目录。
中途失败只会留下暂存件，**不会留下半个状态文件**。

读：核对当前 EUID、目录 0700、文件 0600、单链接、常规文件、≤8 KiB，
读满约定长度后还必须读到 EOF（读取期间被改写即判损坏），最后校验文档内
sha256 校验和。

**一律失败关闭**：半写 / 空文件 / 校验和不符 / 未知版本 / 未知字段 / 超限
都返回错误，**绝不降级成 `Ok(None)`**——否则一次撕裂写入会被洗成"刚刚重置"，
死人开关的情节与冷却状态就凭空消失了。

## 8. 鉴权：不新增无鉴权对外入口

健康页固定挂在 `/api/v1/health`。`local-server` 的 `security::guard`
对 `/api/` 前缀强制走 `auth_http::authorize`，因此会话与 CSRF 语义
与既有内容 API **完全一致**，没有第二套门禁。

装配层只有在**同时**具备会话门禁与证据来源时才挂载该路由
（`crates/local-server/src/health.rs` 的 `enabled`）：
没有会话就根本不存在这个端点，不可能出现"忘了加鉴权"的窗口。

前端同样保守：无法识别的状态值一律降到「未知」；
后端自称 `evidence_complete` 但仍有未知节时，前端不接受完整标记；
读取失败显示「未知」并明写"未知不等于正常"。

## 9. 已知未完成（不得伪称通过）

| 项 | 原因 | 解除条件 |
|---|---|---|
| 告警实际送达 | 发送端未配置，且本轮未装配真实 SMTP 客户端 | 主人给 SMTP 通道，经环境变量注入；再补真实 `deliver` 实现与实机送达验收 |
| 真实恢复点年龄 | 依赖 B1 的 checkpoint／BackupWorker 产出 | B1 落地后由装配层把真实证据接进 `HealthEvidence` |
| 真实磁盘读数 | 需要持文件系统权限的 Broker 公开只读剩余容量查询（Sync 读数已由 Bridge 平面接线，T-132 P1） | Broker 接线 |
| 健康页端到端鉴权验收 | 路由由组合根挂载，需整套装配 | root 接线后跑 local-server 整套契约 |
| 实际两周 headless 试跑 | 只能真实等待 | 不接受模拟时间 |

## 10. 组合根接线（T-048 · W2 追加）

§8 说的"装配层"在这一节落到具体代码。接线只动了四处，每处都在鉴权那一层之内。

### 10.1 路由挂载点

`crates/local-server/src/routing.rs` 的 `assemble`，在 `file_operations::routes`
之后、`security::guard` 那一层**之前**加一行：

```rust
routes = crate::health::routes(routes, state.sessions.is_some());
```

顺序是硬要求：`Router::layer` 只包住调用时已经挂上的路由，
挂晚了就等于新增一个不过门禁的入口。`routes` 对 state 泛型，
不需要改 `AppState`，也不需要改任何公开构造函数的签名。

### 10.2 证据来源的装配（唯一开关）

`health::routes` 不再接收证据来源。证据由可信启动层在整套路由装好之后装上，
与既有 `with_project_repository` 同一种形态：

```rust
// crates/local-server/src/startup.rs
let app = studiolo_local_server::health::with_health_evidence(app, health_input::source());
```

- 装了：健康页给出只读结论。
- 没装：健康页一律 **404**，对客户端而言等同于"这个端点不存在"。

也就是说"页面上有结论"永远等价于"可信启动层确实装配过证据来源"，
不可能出现一个看起来正常的空健康页。`HealthEvidenceSource` 只可能由
`with_health_evidence` 构造，没有第二条路径。

### 10.3 `allow` 表

`crates/local-server/src/security.rs` 的 `allow` match 加入 `crate::health::PATH => "GET"`。
只影响 405 响应的 `Allow` 头，不影响鉴权本身；
`sessions/health.rs::健康页只接受只读方法且405响应的allow头只列出get` 盯住它。

### 10.4 当前每一节证据的真实可得性

组合根（`crates/local-server/src/startup/health_input.rs`）目前只有双向 Bridge 平面（T-132 P1）一个证据源：
装配了它才有第六节、`sync-transport` 活性与 Sync 配额读数；其余各节仍显式"未知"，整体因此仍不是绿。逐条现状：

| 节 | 现状 | 为什么 | 解除条件 |
|---|---|---|---|
| 恢复点年龄 | 未知（"恢复点证据不可读"） | 备份 Worker（B1）未接进组合根，连"有没有恢复点"都读不到；报"尚未建立"是一句证明不了的断言 | B1 接线后由它报 `NotEstablished`，届时才显示 `NO_RECOVERY_POINT` 那句固定中文 |
| 各 owner 活性 | 平面装配后有 `sync-transport` 一个 owner（回执＝最近一轮结束时刻，周期 600 s）；未装配仍未知 | 备份 Worker 的回执仍没接线，所以列表里只有传输 owner，它不会被算成"正常" | B1 接线后加备份 Worker |
| 磁盘配额 | 未知 | 剩余空间只能由持文件系统权限的 Broker 读，`vault-broker` 没有公开的剩余字节查询，HTTP 层自己不许读盘 | Broker 公开一个只读剩余容量查询 |
| Sync 配额 | 平面装配后按镜像字节数对人工配额判定：上限＝审批 `manual_quota.account_limit_bytes`、预警七成，正文带「非账号用量」；未装配或缺人工配额仍未知 | 机器读不到 Obsidian 账号用量，只能拿镜像字节数比人抄的上限（ADR-025 D11） | 无；这就是 D11 的最终形态 |
| 白名单诊断 | 未知 | `host-config` 只公开 `observation_selection`（保护前缀与生成区已被并成一张 `excluded` 表），没有公开人侧可写范围与 `human_content` 登记；用半份快照诊断会凭空造出"搬运范围覆盖保护前缀"这类假冲突 | `host-config` 公开分开的保护前缀／生成区／可写范围／`human_content` 登记 |
| 双向 Bridge | 平面装配后按 D11 十个字段判定，第一批闭合前是未知；未装配即未知 | 契约与字段来源见 `docs/contracts/health-sync-bridge.md` | 无 |

**绝不为了页面好看而假装正常**：`EvidenceSnapshot::unknown` 的每一节都会被只读聚合判成
`Unknown`，整体取最重一节，因此永远不可能显示绿。

### 10.5 前端入口

`HealthBoundary` 挂在**待我动作**视图（`web/src/App.tsx`）——
本机健康本来就属于"待你验收、拍板与重启的事项"，
并且它不依赖工作台快照，快照读不出来时照样能看见。
样式按既有约定新建 `web/src/styles/health.css` 并在 `web/src/main.tsx` 里 import；
三态用局部 CSS 变量着色。390×844 实测 `documentElement.scrollWidth == innerWidth == 390`、
无任何元素越过视口右边界（用例含 96 字符不可断行 ASCII 与超长节标题），故无横向溢出。

### 10.6 为什么本页不写深色分支（与原计划不同，请 root 过目）

`web/src/styles/tokens.css` 用 `color-scheme: light` 把整个工作台钉死为浅色，
**全仓 25 个样式文件里没有第二处 `prefers-color-scheme` 分支**，
`--surface` / `--surface-solid` / `--border` / `--muted` 在系统深色偏好下一个都不变。

本轮初稿曾给 `health.css` 单独加了一组深底配色，实测是**负优化**：
那组色会落在依旧是白的 `--surface`(#fff) 与 `--surface-solid`(#f5f7fb) 上。
WCAG 对比度（0.75rem 状态字属正文字号，门槛 4.5:1）：

| 三态色 | 浅色（现用） | 曾加的深色分支 |
|---|---|---|
| 正常 | `#1f7a4d` 5.32:1 ✅ | `#57c98c` 2.07:1 ❌ |
| 降级 | `#b3452f` 5.51:1 ✅ | `#ef8a72` 2.46:1 ❌ |
| 未知 | `#68717e` 4.94:1 ✅ | `#a6aebc` 2.23:1 ❌ |

所以该分支已删除。**这不是放弃深色**：真要上深色属于 `tokens.css` 的整站改动
（翻 surface/border/muted ＋ 去掉 `color-scheme: light`），不该由一个页面自创一套。
`prefers-contrast: more` 分支保留，因为 `tokens.css` 本来就有同名约定，
且三态色在高对比下达 8.1–9.9:1。
