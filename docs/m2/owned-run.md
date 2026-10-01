# T-048 B3：SyncTransportOwner（OwnedRun）实施记录

2026-09-17。承接 `owned-run-design.md` 与 `headless-execution-contract.md`。
本片交付**纯状态机 + 持久 schema v1 + Linux adapter 代码 + 装配层**；
没有启动过任何第三方程序，没有联网，没有触碰生产服务器或用户 Vault。

## 0. 最重要的一条：受支持状态由 launcher `probe` 实测决定

「本机是否支持受控隔离」不是编译期常量，而是运行时事实（T-132 P1，ADR-025 D2／D3）：

- `linux::verify_backend` 连固定私有 socket `/run/studiolo-sync-launcher/launcher.sock`
  （`protocol::LAUNCHER_SOCKET`），发 `probe`，逐项核对应答：协议版本、nonce 回显、cgroup v2、
  loop quota、专用身份、payload UID/GID 与批准值相等；任一项不成立按其分类报缺口。
  socket 文件不存在＝管理员尚未置备（`TrustedLauncherNotProvisioned`），有文件没人听或超时＝
  `ManagerUnavailable`；非 Linux 仍是 `UnsupportedPlatform`。对调用方而言这些都是 `UnsupportedContainment`。
- `VerifiedLinuxBackend` 仍没有公开构造器、没有 `Deserialize`、没有测试后门：唯一来源是 launcher 的实测应答。
- `LinuxContainment` 十一个方法都是一次 IPC 往返（`linux/client.rs` 是本 crate 唯一的 socket IO 文件，
  架构守卫按精确调用清单登记）；owner 自己不读 cgroupfs、不起子进程。应答种类与请求不符、nonce 不回显
  是 `ProtocolViolation`，身份字段不一致是 `IdentityMismatch`，不猜。
- **绝不回退 PID/PGID 观察冒充 OwnedRun**：`linux::service::is_quiescence_evidence`
  对任何 unit 状态都返回 `false`，并有专门用例锁住这条。
- **CI 只有假 launcher**：契约测试（`tests/owned_run_private/{launcher_fake,client_lifecycle}.rs`）用临时目录里
  同名 socket 上的脚本线程复现线上协议，能证明帧、闭集、`Verdict` 映射与 adapter 的全路径映射，
  **不能**证明 systemd、cgroup、namespace、专用身份或磁盘 quota 真的成立。测试输出里的忽略用例
  「平台契约未验」就是这句话；不得用 skip 冒充绿灯。

`TerminalSummary.container` 记下闭合时的容器身份；`config_output(run_id)` 只对这个身份向 launcher
取 `InspectConfig`／`SetMode` 的 stdout 正文（退出码 0、未超预算、审计已闭合才取，`RunOnce` 永远不取）。

## 1. 最小封闭 API

```text
open_existing(&dyn ApprovedTransport, PinnedRuntime, VerifiedLinuxBackend) -> SyncTransportOwner
inspect() -> RunStatus
prepare_once(expected_sequence, RunOperation, requested_at_millis) -> PreparedRun
pending_audit() -> Option<RunAuditCandidate>
confirm_audit(&RunAuditCandidate, offset, hash)
start(&PreparedRun) -> OwnedRun
cancel_prepared(PreparedRun)
request_stop(&OwnedRun, StopReason)
wait(&OwnedRun) -> RunObservation
recover(run_id) -> RunObservation
stop_recovered(run_id)
quiescent() -> QuiescentMirror
verify_quiescent(&QuiescentMirror)          // 新增：让「作废」可被调用方检验
config_output(run_id) -> String             // T-132 P1：已闭合 InspectConfig／SetMode 的 stdout 正文
```

与设计短文的三处差异，均在此登记：

1. `ApprovedTransport` 在本 crate 是 **trait**；host-config 的封闭能力
   `studiolo_host_config::sync_transport::ApprovedTransport` 实现它。
   这样 owner 不直接依赖配置文件读取，纯逻辑仍可独立验收。
2. `prepare_once` 多两个参数：固定操作与请求时刻毫秒数。
   本 crate **不读时钟**，时间由可信装配提供。
3. 新增 `verify_quiescent`：`QuiescentMirror` 的失效规则（新一轮 prepare、
   根变化、租约变化、owner 重开）必须有一个可调用、可测试的检验入口，
   否则「已作废」只是文档承诺。B4 的执行器消费静止能力时应先调用它。

票据（`PreparedRun`/`OwnedRun`/`RunAuditCandidate`/`QuiescentMirror`）均为私有字段、
无公开构造器、无 `Deserialize`（各自带 `compile_fail` doctest），
绑定 owner 实例随机 epoch、租约、run_id/sequence、批准配置摘要/范围代次、
根 pins、运行时摘要与发放时的持久 generation。

## 2. 阶段持久边界（代码位置）

`BackendDescriptor.service_unit` 是固定可信 launcher 服务的审批值。启动回执里的
`ContainerIdentity.unit_name` 则必须精确等于持久化 sequence/run_id 派生出的
`studiolo-sync-payload-<sequence>-<run_id 前 8 hex>.service`，两者不能混同。
owner 与 launcher 共用 `linux::service::payload_unit_name`；同时继续核对 boot、
cgroup 前缀及真实对象身份、InvocationID、launcher session、隔离与实际预算。

`owned_run/machine.rs` 是唯一的迁移入口；每次转换产出**完整新记录**，
由 `owned_run/session.rs::commit` 原子发布后才生效。

| 阶段 | 边界 | 落点 |
| --- | --- | --- |
| Prepared | state/Intent 已发布，无启动命令；可持久 Cancelled | `machine::prepare` |
| LaunchIssued | **在请求 launcher 之前**发布，并固定当时 boot_id | `lifecycle::start` ① |
| Contained | 固定身份、核对隔离与实际预算之后 | `lifecycle::start` ② |
| ExecutionIssued | **在放行屏障之前**发布 | `lifecycle::start` ③ |
| Running | 来自 `observe_running` 的实际观察 | `lifecycle::next_step` |
| StopRequested | 停止原因**先持久**再调用 `backend.stop` | `lifecycle::request_stop` |
| Quiesced | 入口已关闭 **且** 同一子组 `populated=0` | `lifecycle::next_step` |
| OutcomePending / Closed | 固定 Outcome 保存后补审计；ACK 后原子移入 `last_terminal` | `machine::acknowledge` |

`Intent` 没有 ACK 就 `IssueLaunch` 会被 `AuditPending` 挡下（`machine::step`）。
`start` 借用票据至多放行一次：会话内存标记加上「活动记录已不是 `Prepared`」双重拦截，
重复请求返回 `AlreadyStarted` 且后端 `issue_launch` 计数不变。

停止原因**第一个被持久的就是最终原因**，后续请求不覆盖也不清空；
`RecordExit` 不改阶段、不清停止原因，所以「已取消却 exit 0」保持取消。

## 3. 静止与退出：两件独立的事

`RunObservation` 四项分开报告：`phase`、`quiescence`、`exit`、`stop_reason`。

- 只有 `backend.reap` 真正取得等待结果才写 `Code`/`Signal`，否则是 `NotObserved`。
- 静止只认两种证据（`QuiescenceEvidence`）：
  - `PayloadCgroupUnpopulated`：入口已关闭 **且** 实测子组身份与记录一致 **且** `populated=0`；
  - `BootChanged`：boot 已改变，能证明旧 boot 的进程不存在，但**原退出结果永远 `NotObserved`**。
- `MainPID=0`、`ActiveState=inactive`、读不到 unit、`Child.wait`、PGID、pipe EOF **都不算**。
- 缺 unit / 换 unit（InvocationID 不同）/ cgroup ABA：保持 `ExitUnconfirmed` 语义 ——
  不取 token、不启新运行、`stop_recovered` 拒绝，并且**不假造原成功**。

## 4. 持久 schema v1（`owned_run/state.rs`）

单槽单 writer；`schema_version` 固定 1 且**先单独解析**，未知版本报 `UnknownSchema`
而不是被未知字段吞成 `CorruptStore`。全部结构 `deny_unknown_fields`。

字段：`schema_version`、`installation_id`、`generation`、`next_sequence`、
`binding`（Vault UUID / link / remote / 非零 `scope_epoch` / scope_hash / 批准配置摘要）、
`roots`（规范路径 + dev/ino）、`runtime`、`backend`、`owner`（真实 UID/GID）、
`active`（至多一个 `RunRecord`）、`last_terminal`（至多一个已审计闭合摘要）。

`validate` 会拒绝：代次或序号为零、预算缺项、owner 身份等于 payload 身份、
两个根身份相同、活动记录序号越过 `next_sequence`、活动记录与闭合摘要撞 run_id、
阶段与证据不自洽（Quiesced 无证明、证明未关闭入口、子组空证明不绑定实测子组、
OutcomePending 无 Outcome 槽、Closed 未 ACK、已发启动请求却没有 boot_id）。

单 state ≤ 64 KiB，编解码两侧都检查；不存任何日志正文。计数一律 `checked_add`。

## 5. 唯一 IO 文件与原子替换

`crates/sync-transport/src/store/publish.rs` 是本 crate **唯一**做真实 IO 的文件，
只写三个固定名字：`transport.lock`、`state.json`、`state.json.publish`。

- 逐段 `openat(NOFOLLOW|NONBLOCK|CLOEXEC)` 走到私有根，要求 0700、本人所有；
  固定文件要求普通单链接、0600、本人所有、不超预算。
- 排他租约用 `try_lock`，第二个 owner 得到 `Busy`。
- 发布 = 建临时文件 → `write_all` → `fsync` → `renameat` → `fsync` 父目录，
  前后各复验一次根身份与租约对象身份。
- **残留失败关闭**：`open_existing` 见到 `state.json.publish` 直接 `TemporaryResidue`，
  必须显式调用 `discard_residue` 才能继续。临时文件任何时候都不会被当成授权状态发布。
- `initialize` 是管理员显式动作：三个名字都不存在才建库，并立刻发布首条完整记录。

## 6. 审计

固定 action `sync.transport.run`，Intent / Outcome 两 phase，事件标识与 run/phase 稳定绑定。
payload 只放：schema 版本、run（十六进制）、序号、固定操作、绑定/运行时/后端摘要、
预算、请求主体来源，以及 Outcome 阶段的「是否创建容器 / 停止原因 / 是否实际观察到退出 /
静止是否已证明 / 资源计数」。**没有** stdout、stderr 正文、token、密码、原配置或任意 JSON。

一个活动槽最多两个待补事件（Intent 与 Outcome），且必须按序补齐。
`confirm_audit` 只登记「owner 已核验过的真实 `AuditRecord` 回执」：
它比对候选与当前待补事件完全一致、偏移非零、哈希非全零、偏移严格递增。
**任意 offset/hash 本身不是认证** —— 真实性由 `local-server` 的装配层
（`verify_receipt`：完整性自检 + 记录内事件与本地期望逐字段相等）负责。

## 7. 第一片范围

只开放 `RunOnce` 与同一容器协议下的只读 `InspectConfig`。
封闭 argv 由 `linux::service::build_argv` 生成，固定绝对路径的 Node 与 `cli.js`，
禁 shell、禁 PATH 搜索、禁 `--continuous`、禁任意 flag；
`environment_acceptable` 拒绝 `OBSIDIAN_AUTH_TOKEN`/`NODE_OPTIONS`/`NODE_PATH`/`HOME` 等。
`ApplyApprovedConfig`、登录/配对、Continuous、任意 argv/shell/环境、删除远端**都没有入口**。

## 8. 已验收与未验收

已验收（`cargo test -p studiolo-sync-transport`，服务器上原生跑，全部纯逻辑／临时目录）：

- 行为契约、失败契约、恢复契约三类齐备（假后端驱动纯状态机与持久层），崩溃切口覆盖 Prepared /
  LaunchIssued / Contained / ExecutionIssued 前后、主进程退出但后代未退出、Quiesced 前后、Outcome 与 ACK 前后；
  另有未知 schema、短写、fsync 失败、租约占用、根对象被替换、预算越界等。
- launcher IPC 客户端（`launcher_fake.rs`）：socket 缺失→未置备、没人听→manager 不可用、挂起→5 s 超时；
  帧越界／撕裂／越闭集／种类不明一律缺口；`Refused` 八种 `Verdict` 逐一映射；launch 帧只含闭集键，
  argv 正文与任何路径不进帧；`verify_backend_at` 对七种探测脚本逐项报对应缺口，探测 nonce 每次不同。
- 真实 adapter（`client_lifecycle.rs`）经假 launcher 走完 launch→release→running→drain→reap→gate→events→
  match→stop→config_output 全路径；二次放行（launcher `busy`）固定成 `IdentityMismatch`；`Output` 超预算固定成
  `BudgetUnsupported`；`SetMode` 帧带模式且 argv 摘要独立；`RunSession` 在真实 adapter 上闭合并在
  `TerminalSummary.container` 记下容器身份，`config_output(run_id)` 只对它取正文。

**未验收（需要真实平台）**：真实 launcher 的 systemd transient unit 与 unit 生命周期、cgroup 委派与
`cgroup.kill`、FIFO 屏障与真实 SO_PEERCRED、namespace／挂载视图隔离、专用 payload 身份、loop 镜像磁盘 quota、
真实 Headless 的资源峰值与 200 MB 级边界。CI 只有假 launcher 时**必须报告「平台契约未验」**
（`platform.rs` 的忽略用例把这句话印进测试输出），**不得用 skip 冒充绿灯**。

`local-server` 的 `flush_transport_audit` 循环本身也未运行时验收 ——
它需要一个真实 `SyncTransportOwner`，而那需要真实 launcher 探测通过的 `VerifiedLinuxBackend`。
其中可独立验收的部分（`flush_budget`、`verify_receipt`、`transport_event` 映射、
`status_view` 投影）已有用例覆盖；剩下的是一段直白的循环接线，标注在此等平台就绪后补测。
