# T-046：OwnedRun 实施候选与 Linux 静止证明

2026-09-13；实施候选，尚未实现或启用。承接 `headless-execution-contract.md`；本片不实现 MirrorCAS、不打开 Headless state.db、不推进 Bridge baseline。平台已只读核对：指定服务器为 systemd 259.5、cgroup v2，尚无 Node/Headless/restic；这不等于具备全部隔离条件。

## 结论与可验收切片

交付一个固定 link、一次一个运行的 `SyncTransportOwner`：持久准备 → 真实 Intent ACK → 在已隔离容器内放行固定程序 → 请求停止/等待 → 证明全部 payload 后代退出 → 固定 Outcome ACK → 发仅限本次完整本地观察的静止能力。进程成功退出不称同步成功。重开只观察、停止本运行或补原审计；不得自动重新执行。

第一片生产平台只支持 Linux。建议选 **管理员预置的专用 systemd launcher service + cgroup v2 payload 子组 + 固定可信启动屏障**，不同时实现第二套 PID/PGID 后端。systemd 只管理可信 launcher；payload 子组由可信 launcher 管理，Headless 不能取得其写权限。具体 service、内核能力和 IPC 必须在真实 Linux 上验证后才能声明受支持。没有这些权限的普通 user service 只能返回不支持，不削弱隔离。

这不是复用一套仓库已有隔离器：`runner-adapter/src/lib.rs` 当前只有 Stub、临时结果以及 PID/start-time DTO；`auth-admin/src/process_safety.rs` 只禁 core dump。两者均不证明后代退出。可复用的是 host-config 的私有配置快照、逐段 NOFOLLOW/根身份核验，以及 Broker/Bridge 的排他租约和明确 Unlock 模式；不向新 crate 导出 Broker 原始目录写能力。

## 最小封闭 API（建议冻结）

```text
open_existing(ApprovedTransport, PinnedRuntime, VerifiedLinuxBackend) -> SyncTransportOwner
inspect() -> RunStatus                         // 安全逻辑元数据
prepare_once(expected_sequence) -> PreparedRun // 私有字段，不可构造/反序列化
pending_audit() -> Option<RunAuditCandidate>
confirm_audit(candidate, offset, hash)          // owner 已核验真实 AuditRecord 后确认
start(&PreparedRun) -> OwnedRun                 // Intent ACK + 最终绑定复验后才放行
cancel_prepared(PreparedRun)                    // 仅尚未 LaunchIssued，固定未执行 Outcome
request_stop(&OwnedRun, StopReason)
wait(&OwnedRun) -> RunObservation
recover(run_id) -> RunObservation              // 仅观察旧容器，不重新执行
stop_recovered(run_id)                         // 仅完整匹配旧容器，显式请求
quiescent() -> QuiescentMirror                 // 已有静止证据且 Outcome ACK 才可取
```

`PreparedRun/OwnedRun/RunAuditCandidate/QuiescentMirror` 均绑定 owner 实例随机 epoch、持有租约、run_id/sequence、私有配置摘要/epoch、根 pins、runtime 摘要和持久 generation；只读 DTO 不提供能力。start 借用票据也至多放行一次，重复请求不得二次 spawn。下一次 prepare 先失效旧 QuiescentMirror；根变化、租约变化、owner 重开也失效。已有 token 不能被 Deserialize 或公开 bool 替代。

`RunObservation` 分开报告 `phase`、`quiescence: confirmed/unconfirmed`、`exit: code/signal/not_observed`、`stop_reason`。只有实际取得进程等待结果才称 Reaped；崩溃后仅证实子组为空时写“静止已确认、退出结果未知”。它可在审计闭合后授权重新观察本地镜像，但不是原传输成功回执。无法证明容器身份或创建屏障已关闭则保持 ExitUnconfirmed，既不取 token，也不启新运行。

第一片只放 `RunOnce` 与同一容器协议下的只读 `InspectConfig`；后者是独立串行运行，不提供静止后的配置修改入口。`ApplyApprovedConfig`、登录/配对、Continuous、任意 argv/shell/环境、删除远端不开放。先用固定合成程序测完整 lifecycle，再用精确发布的配置查询对齐参数；第一片不需要真实账户。

## 持久 schema v1：独立受控 FS，一槽一个 writer

新增显式配置的 transport-private 根，与 canonical、writer-data、Auth/Audit、Bridge DB、镜像、Headless config、外 stage 两两不重叠。只有 owner 进程写运行日志；launcher 不打开该日志，只通过窄 IPC 提交事实。初始化是管理员显式动作；open_existing 不补目录/空 state，不自动认领残留。新 crate 不加 SQLite，不改 BridgeState 或 Broker 主日志版本。

固定 `transport.lock`、`state.json` 与单个已登记发布临时文件；全部普通单链接、私有权限、身份验证、目录 FD 相对读写，文件与父目录 fsync。采用完整原子替换：state 永远只接受旧版或新版完整记录；未知版本、缺失启用后的 state、半 JSON、未知字段、错绑定、临时残留与缺失正式记录一律失败关闭。不能把临时文件当授权状态自动发布。单 state 最大 64 KiB，不存日志正文。每次读取前后复验 pins/generation，计数 checked_add。

| 字段 | 精确定义 |
| --- | --- |
| schema_version | 固定 1，显式解析；后续版本不得回写降级 |
| installation_id / generation / next_sequence | 显式建库随机 ID；单调提交代次；已分配序号永不复用 |
| binding | canonical UUID/pair、link ID、remote ID、scope_hash、非零 scope_epoch、批准配置摘要；这些是请求绑定而非认证 |
| roots / runtime / backend | 根规范路径及 dev/ino、运行 UID/GID、固定 Node 与完整只读安装闭包摘要、backend 配置摘要；不保存凭证正文 |
| active | 至多一个 RunRecord；不能因进程句柄丢失而清空 |
| last_terminal | 至多一个已审计闭合摘要，供最近状态；完整历史在 Audit，不做无限目录扫描 |

RunRecord 固定 `run_id`、sequence、operation（inspect_config/run_once）、请求时刻、预算、Intent 事件/ACK、phase、单调 stop_reason、容器身份、原始退出结果（可未知）、静止证明摘要、Outcome 事件/ACK。容器身份含 boot_id、精确 unit 名、manager InvocationID、payload cgroup 路径及 dev/ino、一次性 launcher 会话 ID。PID/start-time 仅诊断，不能代替上述身份或授权 kill。root 与 child 传递固定 nonce 用于本次握手，不作为长期认证密钥，不写入公开 DTO。

| 阶段 | 持久边界与重开处理 |
| --- | --- |
| Prepared | state/Intent 已可靠发布，无启动命令；可以持久 Cancelled。Intent 没有 ACK 不启动 |
| LaunchIssued | **在请求启动 launcher 之前**持久，之后任何 IPC/启动错误不能直接声称从未执行；原槽保留 |
| Contained | 屏障内可信 launcher 已建立 payload 子组，owner 已固定身份并核对隔离/预算；未放行 Headless |
| ExecutionIssued | **在放行屏障之前**持久；通信丢失不能重新发送产生第二次执行，launcher 协议也只消费一次放行 |
| Running / StopRequested | Running 来自实际执行观察；停止原因先持久，取消优先于后来 exit 0；不把已请求 stop 清回 Running |
| Quiesced | 已关闭继续创建 payload 的路径，且同一个受控 payload 子组 populated=0；实际 wait 结果独立保存 |
| OutcomePending / Closed | 固定 Outcome 可靠保存后可补审计；ACK 前不启新运行/不发静止 token；ACK 后原子移入 last_terminal |

启动前后错误公开为固定分类，不带原 JSON、凭证、原始 stderr。未知不是永久“失败成功二选一”：保留活动槽与证据，通过 recover/stop_recovered 继续查同一容器。若已重启 OS，boot_id 改变能证明旧 boot 的进程已不存在，但不能证明原 exit/同步结果；需显式恢复审阅、复验全部根、新 boot 容器空闲与启动入口关闭、持久“旧退出结果未知”的终态，审计 ACK 后才重新观察/运行。不得仅以读不到旧 unit 自动认定静止，也不得拿新的同名 cgroup 冒充旧的空组。

## Linux containment 的实施边界

1. **管理员预置一个固定 link 的启动服务。** service 模板、launcher 二进制、Node/安装闭包、只读最小 root image 与权限策略均固定、不可由 payload 修改。owner 只准控制自己生成并已登记的 run 实例，不接任意 unit/属性/命令。Start/Stop 查询通过独立 `linux/service` adapter 调本地 systemd 接口；若采用 CLI，也只能固定绝对路径、封闭 argv、无 shell/环境继承，CLI 退出码不能当 payload 退出证明。实现时选一种接口，不留运行时任意切换后端。
2. **可信启动屏障避免 spawn→迁入 cgroup 的竞态。** launcher 先处于受控服务；在执行第三方 Node/JS/native code 前创建并绑定 payload 子组，建立 mount/PID 等隔离、降为专用 payload UID、丢弃 capabilities，给 owner 返回真实身份和策略检查结果。owner 固定 cgroup FD 并持久 Contained/ExecutionIssued 后才发一次放行。payload 由可信启动路径直接在最终组内启动；禁止先执行不可信程序再迁 PID。最终 mount 视图内也须核对镜像/config/runtime 的真实身份与批准映射，防止只验宿主路径后重新按已漂移的字符串 bind。launcher IPC 用私有根固定端点、双方实际 peer credentials、run ID/nonce 和有界帧（≤16 KiB）；不传 secret、正文、任意路径或 executable。
3. **payload 与 launcher 权限分开。** 只向可信 launcher 委派子组管理；payload 无 cgroupfs 写权限，不得访问 manager socket/launcher IPC/owner FD，不能 unshare/setns/mount/ptrace 脱离边界。专用 payload 身份无补充组、无宿主 owner 权限。挂载视图只含固定只读运行时、所需系统库/证书、专用可写镜像和 Headless config、有限 tmp；没有 canonical、writer-data、Bridge DB、Auth/Audit、外 stage或宿主 home。仅 ProtectSystem/read-only 全盘不是“不可读取宿主秘密”，不能当最小文件系统隔离替代。
4. **只证明 payload 子组空，不把仍活着的可信 supervisor 算入 payload。** launcher 持有上层 service cgroup，负责保留子组直到 owner 确认终态；它不能在单次放行后创建第二次 payload。Quiesced 前必须关闭/证实关闭一切继续 spawn 的入口，再核对固定子组 `cgroup.events populated=0`。保持该子组真实 FD/identity 与同一个 InvocationID 直至本地持久终态；service 禁自动重启，payload 和非批准用户无外部 start/restart/属性修改权限。不能依赖 systemd 自动回收后查询“找不到”。`MainPID=0`、`ActiveState=inactive`、Child.wait、PGID、pipe EOF 都不是此证明。
5. **stop 仅作用于绑定的子组。** 先持久请求，TERM grace 有界；强停须验证同一受控子组后使用覆盖该组所有后代的机制（例如可用的 cgroup.kill）。不得进程名搜索/杀别的 Headless，不删 .sync.lock，不用全主机 killall。Audit 暂不可用不能阻止停止已经运行的自有 payload；安全停止事实进入本地槽待补。
6. **资源规则必须可核验而非仅已请求。** 放行前读取实际 memory.max、memory.swap.max、pids.max、cpu.max 与策略预期；固定 wall deadline、TERM grace、stdout/stderr 字节限额和配置 JSON 64 KiB 上限。stdout/stderr 持续有界排空，仅保存计数/固定分类，超限停止，不能截断后按成功解析。Headless 内部日志、state、mirror/tmp另需受控文件系统 quota/硬上限；cgroup 不限制磁盘，stdout 限额不限制 sync.log。预算缺项或实际配置不支持则不启动。
7. **权限不足的固定结果。** 首次检查缺 cgroup v2/委派权限/控制器/kill能力、namespace 或专用身份隔离、磁盘 quota，返回 `UnsupportedContainment`，不启动第三方程序。如果执行请求已经发出才丢失权限/manager/cgroup身份，保持 `ExitUnconfirmed` 和原槽，禁止新运行/静止能力；只可观察或在身份重验成功后停止原容器。绝不回退 PID/PGID；mock 返回值不能把平台标为可用。macOS 首片同样明确 Unsupported。

主线本轮已核对官方资料：[kernel cgroup v2](https://cdn.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html) 的 populated 覆盖本组及后代的 live 进程；0 不包括 zombie，但 zombie 已不能继续写。实际回收/退出码仍单列，不能从 populated 补造。官方 systemd `systemd.kill.xml` 说明 `KillMode=control-group` 停止整组剩余进程，而 process/none 可能遗留；本设计既要求该策略，也独立读取真实 populated，不只信 inactive/PID 0。这是主线提供的本轮来源核验，本子线未联网或动态验证。

启动屏障和持久槽并不冻结其它具备 OS 特权的管理员。特权改根/改 service 会使后续复验拒绝；本文不承诺抵御能修改本进程和 kernel 的主机管理员。远端设备仍可并发写入；本地静止 token 不提供远端静止或完整覆盖。

## root 先登记的文件域、IO 与审计

| 登记点 | 最小变化与允许 IO |
| --- | --- |
| 新 `crates/sync-transport` | 专用 owner，依赖 core 与 host-config 的批准能力；不依赖 RunnerAdapter/TaskEngine。`owned_run/{types,state,machine}.rs` 纯 DTO/状态迁移；`store/{paths,publish}.rs` 只写固定 transport-private；`linux/{service,launcher,cgroup,ipc,stdio}.rs` 各自白名单 OS/进程/IPC 权限；不导出 raw FD、通用 exec 或写回调 |
| `crates/host-config/src/sync_transport*` | 显式默认关闭，审批 runtime/service/root/UID/预算并封闭成能力；延续 SyncScope 的快照复验，scope 与真实根 authority 分开。既有配置全文哈希不改语义 |
| `crates/local-server/src/sync_transport*` | 唯一组合 owner、真实 Auth/配置新鲜度、AuditLedger 映射；现有纯 `sync_transport_config.rs` 可保持或移动为同语义纯模块；配置查询不能自行取得进程 IO |
| `crates/studiolo/tests/architecture_support/{policy,source}.rs` 与新专项 | 登记新 crate 依赖与逐文件 IO，不把整个 crate 豁免；补假命令、任意 fs/数据库/网络写入毒针。现 `sync_bridge.rs` 守卫继续禁止 planner/SQLite 发起 Command |
| 部署档案/权威写路径表 | 明确 systemd launcher、payload cgroup、IPC、transport-private 的唯一 owner与 OS 准入；显式 enable/init 动作，不让启动隐式创建批准目录 |
| 敏感动作矩阵/Audit | 新固定 action 建议 `sync.transport.run`，phase Intent/Outcome，payload schema_version=1；不要混用 `bridge.action`、文件 mutation 或声称远端送达 |

审计 Intent 固定 run_id/sequence、绑定摘要、固定操作、runtime/backend 摘要、预算与请求主体来源；Outcome 固定创建/停止原因、退出是否实际观察、静止证据摘要及固定资源计数。事件 ID 与 run/phase 稳定绑定，不放 stdout/stderr、token、密码、原配置、任意 JSON。新 crate 仅给封闭候选，root 追加固定 AuditEvent 并核对完整记录后确认；任意 offset/hash 本身不是认证。一个活动槽最多两个待补事件，启动前预留二者容量。Outcome 待 ACK/原进程未静止均挡下次运行；本片并不解除 Bridge sticky fuse 或旧 action pending。

## 真实测试环境与最小验收（root 串行执行）

需一台隔离 Linux VM/测试机：实际 systemd、cgroup v2 与受控委派、专用 payload UID/GID、可核验的最小 root image/mount 隔离、可执行 cgroup kill、磁盘 quota及足够内存。CI 只有 mock/无权限时应报告平台契约未验，不能用 skip 冒充绿灯。无需真实账号和真实 Vault；先用固定合成 helper 做以下进程事实测试。

- 子进程 fork/双 fork/setsid，主进程 exit 0但孙进程仍写镜像：没有 token；只杀自有容器，外部哨兵不受影响。payload 尝试写 cgroup/访问 manager、canonical/host home/外 stage均拒绝。
- 崩溃点：Prepared、LaunchIssued、Contained、ExecutionIssued前后、主进程退出/后代未退出、Quiesced前后、Outcome/ACK前后。重开不二次 exec，仍可停止旧容器；已取消却 exit 0保持取消；缺/换 unit、cgroup ABA、boot变化不假造原成功。
- cgroup写权限不足、控制器设置未生效、manager断开、输出洪水/卡住的pipe、超时、OOM、磁盘 quota满：启动前拒绝或启动后原槽未知；没有 PID 降级/先发 token 后报错。
- 把 stage/镜像根替换、配置范围变化、runtime/helper换 inode、跨实例票据、重复借用、父进程 FD副本跨 child：都不增加权限、不重执行、不遗留 close-only 租约。
- 同一 retained payload cgroup 的空证明 + spawn入口关闭，随后完整镜像观察才可取得本地证据；再 prepare即撤销旧 token。mock/纯状态机测试另覆盖未知 schema、短写/fsync失败、audit失败与全部阶段持久化。

真实 Headless 本地 version/config-query 验收和资源峰值是后续同片平台验收；带账号 RunOnce、独立设备精确字节读回、协商 perFileMax 与两周试跑另外记录。内部对象上限256MiB/暂存512MiB不是 Plus 服务承诺，完整本地扫描也不是远端全量 manifest。本设计没有测试或证明这些外部状态。

## 证据路径与需 root 拍板的唯一新增前提

依据当前仓库：`docs/m2/headless-execution-contract.md`；`crates/local-server/src/sync_transport_config.rs`；`crates/host-config/src/sync_scope.rs` 与 `config_file.rs`；`crates/vault-broker/src/writer/lease.rs`；`crates/sync-bridge/src/sqlite/{opening,audit_events}.rs`；`crates/runner-adapter/src/lib.rs`；`crates/studiolo/tests/architecture_support/{policy,source,sync_bridge}.rs`。均只读，未将旧方案当已实现事实。

需要 root 先批准并登记 **Linux 专用可信 launcher/service 的特权与部署方式**，以及真实测试环境。它是当前仓库缺失的必要隔离能力，不能省掉后用 PID 包装成 OwnedRun。具体 systemd/IPC 库选择属于实现依赖冻结，必须先核对本地部署版本和已有锁缓存；本设计未联网核实可用版本，不建议在本轮顺手引入通用 Runner 或完整容器平台。其余状态机、单槽 schema 与授权边界可先按本短文实施纯契约，真实“受支持”状态仍以 Linux 实测为准。
