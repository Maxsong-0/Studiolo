# T-132 P1：双向 Bridge 驱动平面设计（ADR-025）

2026-09-23。承接 `owned-run.md`、`bridge-executor.md`、`bridge-state.md`、`sync-scope.md` 与
Vault `04-协调/2026-09-22-T-132-双向Bridge上线方案.md`（P1 第 1–5 条、第 6 节、拍板 Q1～Q4）、
`2026-09-22-T-132-P2-试跑记录.md`（步骤 5 结论 1～9 是硬要求）。决定 D1～D15 的取舍见 ADR-025。
本页是 A 轮地基的权威说明；launcher 协议单独在 [`sync-plane-design-launcher.md`](sync-plane-design-launcher.md)。

## 1 拓扑与进程边界（D1／D2／D7）

```text
studiolo-human（uid 1000）                      root                          studiolo-sync（uid 999）
┌───────────────────────────────┐   IPC   ┌────────────────────────┐  systemd-run  ┌──────────────────────┐
│ sync_plane（驱动循环，300 s）  │────────▶│ studiolo-sync-launcher │──────────────▶│ 垫片 payload → exec  │
│  ├ SyncTransportOwner         │  socket │  serve                 │  transient    │  node cli.js sync    │
│  ├ BridgeState（sqlite）      │◀────────│  session 状态机        │  service      │  只见镜像根+配置根   │
│  └ coordinator::sync_bridge   │  帧     │  FIFO 屏障／statfs     │◀─ report FIFO ┤                      │
└───────┬───────────────────────┘         └────────────────────────┘               └──────────┬───────────┘
        │ idmapped bind 视图 /var/lib/studiolo-owner/mirror                                   │ 原地写入
        ▼                                                                                     ▼
   /var/lib/studiolo-sync/mirror（ext4 loop 镜像 /var/lib/studiolo-sync.img，4 GiB，quota 依据）
```

- 平面住在 `crates/local-server/src/sync_plane/**`，组合根 `startup/sync_input.rs`，`startup.rs` 一处调用；
  形态照 `plane_input::board`。**默认关闭**：无 `--sync-transport <approval.json>` 即 `Ok(None)`；
  有审批文档但私有配置缺 `sync_scope` → `StartupError::SyncPlane`（不静默）。
- launcher 是新 crate `crates/sync-launcher`（lib + bin `studiolo-sync-launcher`，依赖只有 sync-transport、core）。
  `serve`：root 系统服务，socket `/run/studiolo-sync-launcher/launcher.sock`，目录 0750 root:studiolo（owner 组），
  只接受 SO_PEERCRED uid==owner uid。`payload`：垫片，在 transient unit 内以 `studiolo-sync` 身份运行。
- owner 进程自己不读 cgroupfs、不起子进程：`LinuxContainment` 每个方法都是一次 IPC 往返（D3）。
- 镜像视图：owner 只经 idmapped bind 视图读写（审批 `owner_view_root`，Rust 访问器叫 `mirror_view_root`——
  local-server 的写者边界词法门禁禁止任何 `.owner` 记号）。`RootPins.mirror_root` 是 payload 视图内的真实路径，只进 argv；
  `mirror_identity` 经视图观察（bind 同 inode），launcher 在最终视图内 stat 真实根后必须报回同一 `(dev,ino)`。
- 新私有根 `plane_root`（审批文档新增）：`stage/`、`plane-state.json`、`plane.lock`、`bridge/`（BridgeState）。
  与 transport 私有根、视图根、canonical、writer-data、Auth/Audit 两两不重叠（`ApprovedTransport::approve`）。
  **必须与 `owner_view_root` 在同一条 vfsmount 上**：安装是 stage 对象 `renameat` 进镜像，跨挂载必 EXDEV；
  置备把 `bridge/{mirror,plane}` 作为一个父目录 idmapped bind 到 `/var/lib/studiolo-owner/bridge`，
  `mirror::install` 安装前用 `statx(STATX_MNT_ID)` 核同挂载，不同即 `Storage`（C 轮按 W4 回执补）。

## 2 一轮驱动的精确顺序（`sync_plane/driver.rs::run_round`）

| # | 步骤 | 失败即停在哪 | 重开怎么续 |
|---|---|---|---|
| 1 | `recovery::PlaneStateStore::open` → `decide(state, 审批模式, now)` | 状态文件撕裂→`Storage`，本轮不跑 | 下一轮重读；撕裂须管理员 `sync-init` 重建（不自动重置） |
| 2 | `flush_transport_audit`；活动槽存在则 `recover(run_id)`；未闭合→`Transport`。关停标志（`drive_until` 收到停止信号）只在安全停点查：InspectConfig、SetMode、「已启动」之前，RunOnce 回收落盘之后，取票、观察、执行之前 | 保留原槽；关停即 `Stopping`，执行器内部不打断 | 每轮先补审计、再 recover，直到 Closed；关停后不再起新轮 |
| 3 | `run_headless(InspectConfig)` → `config_output` → `headless::classify` 依次核对 ① 审批母本＋应处模式 ② 母本＋另一模式 ③ 母本∪`pending_purge`＋应处模式 ④ 并集＋另一模式（③④ 只在 `pending_purge`＝上次核验集合−审批母本非空时试）；`fileTypes` 含五类等其余项逐项==审批。命中的那份记为 `headless_excluded`，与 `plane-state.json` 所记不同即先落盘（守卫拦住的轮次也留下，供 purge 核对） | 四种都不一致→`ConfigMismatch`，健康降级 | 模式不符走第 5 步；排除项／类型不符只能人改，每轮重核 |
| 4 | 在 InspectConfig 闭合留下的静止票据下 `recovery::residue_guard(上次核验集合, 审批母本, 视图根)`，读完复验票据；残留已清但 `pending_purge` 非空→`ConfigMismatch`（headless 还排除着审批已去掉的文件夹，安全顺序进行中） | 有残留→`ResidueBlocked`，固定原因「排除项清除前有镜像残留」；票据作废→`Quiescence` | 安全顺序（部署手册第 8 节）：停 human → `sync-purge-residue` → headless 侧清排除 → 起 human，下一轮自动放行 |
| 5 | 需要时 `run_headless(SetMode(effective_mode))`，回到 3、4 按同样逻辑复核 | SetMode 非干净闭合→`Headless`（owner 关停时被停→`Stopping`） | 下一轮重试；`applied_mode` 只在复核通过后写 |
| 6 | `mark_started(seq)`（持久「已启动」）→ `run_headless(RunOnce)`（轮询中关停标志为真即 `request_stop(OwnerRequested)`，照样轮询到闭合）→ `mark_reaped(clean)` 落盘；非 clean 记退避 | RunOnce 非 0／未静止→本轮停，`not_before=now+5s`（rc=1 连续按 5/10/…/300）；被关停→回收先落盘再报 `Stopping` | 断电或被停：`started_sequence` 仍在或上次未干净→下一轮强制 `MirrorRemote` 恢复一次 |
| 7 | `OwnerQuiescence::take`（`quiescent()`，窗口 1800 s） | 无静止证明→`Quiescence` | 下一轮重跑 RunOnce |
| 8 | `observe_both`：canonical 经接缝完整观察，镜像经视图完整观察（镜像观察、搬运读、核对读每次都先 `quiescence::verify` 复验第 7 步的票据），`approve_union` → `Input` | 任一侧不完整／范围未批→`Observation` | 下一轮重观察 |
| 9 | 观察后一律 `plan` 试算。MirrorRemote 有冲突→`preresolve_conflicts`（D6）→ 重做第 8 步一次；Bidirectional 有冲突（含熔断冻结的批）→ `driver/preserve.rs` 先把戳写进 `plane-state.json`，再把双方现存版本各保全一份到 `ops/bridge-conflicts/`，然后照常执行（执行器对冲突路径不动、闭合时不推进基线）；同时 `conflict_copy_count` 数出两侧观察里的冲突副本回填健康 | 预解决删除越熔断→`Fused`（不解决，健康降级）；保全写入失败→按接缝分类停在执行之前 | 预解决：人在界面选边（后续任务）；保全：下一轮沿用已落盘的戳、同一路径与幂等键续写，不另生一份 |
| 10 | `bridge.reserve(version, input)`：Fused→`Fused` 停；Ready→`StateJournal`＋`execute(plan, ports, now, deadline)` | 逐动作 Issued 后未知→`Unknown`，本批不闭合 | 下一轮第 10 步 `reconcile::reconcile_pending` 只取仍未知的原键：canonical 侧由冻结前置直接还原原 `Revision`（Bridge 证据的 `revision` 就是它的 32 字节原值，`driver/evidence.rs::revision_from_digest`），经 `PlaneJobBackend::canonical_verdict` 按原键重建首次请求走接缝——有原回执照收（接缝绝不二次安装）、无回执才由接缝首次安装，CAS 拒绝→NoEffect，`NotAttempted` 且写者此刻接得上（原键已取消／受保护路径／预算）→NoEffect；镜像侧在同一视图的静止票据下逐字节比对目标：等于冻结源→Committed，Delete 且已不存在→Committed，仍是冻结前置（CreateOnly 即不存在）→NoEffect，读完复验静止失败则作废；核出的经 `StateJournal::record_outcome` 写回执，其余保持未知、本批不闭合。已安装的 canonical Replace／Delete 同样按原键核对（前置跨轮可还原，2026-09-26 起）；但入站源在镜像上已变或读不出时重建不出首次请求（内联写的指纹含正文），只能保持未知、不能记成无效果（评审 2026-09-28 P1-1；让核对不依赖镜像当前内容另立任务） |
| 11 | `execute` 内部：Intent 审计→逐动作 保全→Issued→apply→回执→Outcome/Summary 审计→settle→退休 | 任一审计推不动→`Audit` | 下一轮 `deliver_audit` 继续 |
| 12 | `stage.sweep(now, retired_through)`：保留期到且计划已退休的保全对象回收；孤儿回收 | IO 失败→`Storage`（不影响本批结论） | 下一轮再扫 |

`execute` 的端口：`quiescence`=`OwnerQuiescence`，`journal`=`StateJournal::new(&mut bridge, &*audit, input)`，
`witness`=`PlaneWitness`，`content`=`PlaneContent`，`sink`=`StageSink`，`backend`=`PlaneJobBackend`，`cas`=`MirrorCas::new(CasLimits::internal())`。
`deletion_fuse_blocked` 在 `driver.rs` 复写同一判据（sync-bridge 不在本轮写域），W3 用契约测试证明与 `plan` 同结论。

## 3 launcher 协议

见 [`sync-plane-design-launcher.md`](sync-plane-design-launcher.md)：请求／应答 JSON 字段表、`Verdict` 闭集与
`ContainmentGap` 映射、session 状态机、超时。DTO 母本在 `crates/sync-transport/src/linux/protocol.rs`（+`protocol/response.rs`），
顶层键闭集 `ipc::ALLOWED_KEYS`（44 键）；launcher 的 `protocol.rs` 只做重导出与编解码入口。

## 4 引导期与 P6 语义（D5／D6）

- 规划与执行**始终双向**（`plan`／`BridgeState` 冻结语义不改）。引导期安全全靠 headless 处于 `mirror-remote`：
  Bridge 出站装进镜像的改动会被下一次 RunOnce 回滚，再按三方基线规划为入站；canonical 因此自然不是权威。
- MirrorRemote 下每条**出站**动作前，`PlaneJobBackend::apply_mirror` 先把 canonical 当前版本经接缝创建到
  `ops/bridge-rollback/<原路径去扩展名> (Conflicted copy studiolo-canonical <UTC yyyyMMddHHmm>).<ext>`（主体 Obsidian，幂等键 `<external_key>/rollback`）。
- MirrorRemote 下的冲突（FirstDivergence／BothChanged／DeleteAgainstModification）在下一轮规划前按「镜像为准」预解决：
  canonical 版本保全到 `ops/bridge-conflicts/…`，canonical 路径 CAS 到镜像版本（`Match(观察到的 revision)`）；镜像缺失则条件删除（走文件操作协议、进墓碑），
  删除仍受熔断（`deletion_fuse_blocked`）。首次对齐（canonical 207 份 vs 镜像无共同基线）就是这条的普通情形。
- Bidirectional（P6）下冲突不自动解决：观察后一律试算，有冲突（含熔断冻结里的冲突）先把双方现存版本各保全一份到
  `ops/bridge-conflicts/`——canonical 版本用设备名 `studiolo-canonical`、镜像版本用审批 `device_name`，命名同
  `naming::copy_path`，条件 `MustNotExist`——再照常执行（执行器对冲突路径不动、闭合时不推进基线），等人在界面选边
  （`driver/preserve.rs`）。幂等：冲突身份＝sha256(域‖link_id‖路径‖两侧摘要或 32 个 0)，首次保全的 UTC 分钟戳记在
  `plane-state.json` 的 `conflict_stamps`，戳先落盘、后写副本，同一冲突跨轮总落在同一副本路径与幂等键
  `conflict/<link>/<身份>/<戳>/{canonical,mirror}` 上：有原回执即命中，副本已在（回执不在）即视为已保全，不覆盖、不重复。
  冲突解决或任一侧版本变了即回收其戳；原样重现按新戳另存一份。一轮至多保全按路径序的前 512 条。计数见第 6 节。
- **P6 前必须解决的风险**：整批基线不前进的规划器限制（`settle` 只在无冲突、全成功时推进基线）在 P6 会造成
  「一个未解冲突冻结全库基线」——所有其他路径的共同观察也不会进基线，下一轮仍按旧基线规划。P6 开工前需在
  sync-bridge 加「冲突路径排除后推进基线」的显式入口（另立任务，属 sync-bridge 写域）。
- `_规则/` 在 `sync_scope::paths::hard_excluded` 里是硬排除；D2 拍板的范围里 `_规则/` 本轮**进不了** SyncScope，见 ADR-025 未决。

## 5 持久文件清单（所有者、权限、预算）

| 固定名 | 所有者／权限 | 预算 | 写者 |
|---|---|---|---|
| `<transport_private>/{transport.lock,state.json,state.json.publish}` | owner 0600／根 0700 | 64 KiB | sync-transport `store/publish.rs`（不变） |
| `<plane_root>/plane.lock` | owner 0600 | 4 KiB | `sync_plane/recovery.rs`；flock 按打开文件描述计，租约守卫析构时显式解锁（并发 fork 在 exec 前继承的描述符不延长租约） |
| `<plane_root>/plane-state.json`（+`.new`） | owner 0600 | 64 KiB，版本 1，校验和；追加字段 `headless_excluded`（headless 最近一次核对报的排除集合）、`conflict_stamps`（冲突身份→首次保全分钟戳，≤512 条）、`pending_purge`（清除审计日记），为空时不写出、非空才进校验和，旧文件照常解码 | `sync_plane/recovery.rs` |
| `<plane_root>/stage/objects/<sha256 hex>` | owner 0600 | 单对象 256 MiB，合计 512 MiB，4096 项（含孤儿） | `sync_plane/mirror/stage.rs` |
| `<plane_root>/stage/index.json`（+`.new`）、`stage/objects/{receiving.partial,installing.partial}` | owner 0600 | 1 MiB | `sync_plane/mirror/stage.rs` |
| `<plane_root>/bridge/{bridge.sqlite3,bridge.lock}` | owner 0600 | 主库 256 MiB | sync-bridge `sqlite/storage.rs`（不变） |
| `/run/studiolo-sync-launcher/launcher.sock` | root:studiolo 0750 目录，socket 0660 | 帧 16 KiB | launcher `serve.rs` |
| `/run/studiolo-sync-launcher/sessions/<session hex>/{release.fifo,stdout,stderr}` | root 0700 | stdout/stderr 按 RunBudget（报告帧在 stdout 头部） | launcher `unit/session.rs` |
| `/etc/studiolo/sync-launcher.json` | root 0600 | 16 KiB | 管理员（provision.sh） |
| `/var/lib/studiolo-sync.img` + mount 单元；`/var/lib/studiolo-owner/mirror` idmapped 视图 mount 单元 | root | 4 GiB | provision.sh |
| `ops/bridge-conflicts/`、`ops/bridge-rollback/`（canonical 内，范围外） | Broker | 受 Broker 限额 | 经接缝，主体 Obsidian |

## 6 健康证据字段（D11，`observability::health::BridgeEvidence`）

节 `sync-bridge`／「双向 Bridge」：`last_batch{sequence,unix_seconds}`、`fused`、`unknown_jobs`、`quiescence_valid`、
`conflict_copies`、`mirror_bytes`、`last_exit`（clean／non_zero／signal／deadline_exceeded／owner_stopped／unconfirmed）、
`recovery`（idle／forced_mirror_remote／backoff）、`residue_guard`（clear／blocked）、`mode`。任一 `None` → 未知；
`fused==true`／`unknown_jobs>0`／`residue_guard==blocked`／`last_exit≠clean`／`recovery≠idle` → 降级；`quiescence_valid==false` → 未知。
`conflict_copies`＝最近一次完整观察里两侧命名为冲突副本的路径数（headless 与 `studiolo-canonical` 两种设备名都识别）
＋持久 `conflict_stamps` 当前条数（含正在续写副本的冲突，非历史副本文件总数）；`owner_stopped`＝headless 在 owner 关停时被 `request_stop` 停下。
平面未装配（默认关闭）时本节也是未知：与恢复点、白名单两节同一口径，没启用的能力不显示绿。
`owner-liveness` 加 `sync-transport`（回执＝最近一轮时刻，周期 600 s）。`sync-quota`：`SyncUsage` 取镜像字节，
上限＝`manual_quota.account_limit_bytes`，预警＝七成；detail 固定带「非账号用量」；`manual_quota` 缺→未知。
前端契约 `docs/contracts/health-sync-bridge.md` 由 W5 写，本轮不改 `web/**`。

## 7 守卫登记清单（D15，全部由 W1 收口）

| 位置 | 登记 |
|---|---|
| `architecture_support/policy.rs` | `studiolo-sync-launcher => [sync-transport, core]`；local-server 加 `studiolo-sync-bridge` |
| `architecture_cases/dependencies.rs` | crate 数 24；rustix 归属加 `studiolo-sync-launcher` |
| `architecture_support/sync_launcher.rs` | `config.rs`／`serve.rs`／`payload.rs`／`unit.rs`／`unit/reads.rs`／`unit/session.rs` 六文件的精确调用清单（按 W2 回执） |
| `architecture_support/sync_transport.rs` | `linux/client.rs`：`UnixStream::connect(socket)`、`stream.write_all(&frame)` 各一次，`read_exact` 两次，`std::os::unix::net::UnixStream` 导入一次 |
| `architecture_support/sync_plane.rs` | `stage.rs`（openat／fstat／fsync／renameat／unlinkat／mkdirat／Dir::read_from 各一、statx 两次）／`recovery.rs`（openat／fstat 各一，flock 两次：加锁与租约守卫 `Lease` 析构时显式解锁）／`mirror.rs`（renameat／fsync 各一）的固定名锚点＋精确调用清单（按 W4 回执） |
| `architecture_support/maintenance.rs` | `maintenance/sync.rs`：两条导入行＋九处固定调用；`sync_rebind.rs` 仅一次固定文件存在性探测 |
| `architecture_cases/canonical_owner.rs` | `coordinator/sync_bridge.rs` 的八条允许形态（含流式原键取回执）（第 4 节） |

## 8 未决

应为空。三条默认选择记在 ADR-025「未决」：`_规则/` 硬排除、`TerminalSummary` 不带容器身份（`config_output` 依赖 W1 补字段）、
预解决删除的熔断判据在 driver.rs 复写。

### 关停与启动恢复

关停信号先通知 Bridge 并等待其安全停点收尾，期间 canonical 写者保持可用；随后停止 workbench、排空 HTTP，最后等待写者的关闭清理完成。驱动异常退出也走 HTTP 排空。启动时从传输存储的 `last_terminal` 恢复退出分类（含 `OwnerStopped`），从平面持久 `conflict_stamps` 恢复冲突数，不伪造新一轮活性回执或静止票据。

### 入站源与审计恢复（P6 前遗留）

canonical 入站第一次写前，先在有效静止票据下逐帧读取冻结镜像版本，核对长度与摘要，封口外 stage 对象并持久登记计划序号。首次缺源或登记失败为 NotAttempted；已 Issued 动作重开只读保全源，缺源／损坏保持 Unknown。小对象始终整篇写，大对象始终流式写；原路径、前置、幂等键和请求类型不变，流式接缝先查原回执，已有回执不二次安装。

同摘要引用的保全时刻和计划序号分别取最大，只有保留期到且所有引用所在计划已退休才可回收。镜像安装遇到被引用对象时，逐帧制作独立安装副本，不能 rename 消耗保全源。实际磁盘对象（含孤儿、接收与安装临时件）共同受 4096 项／512 MiB 限额，单对象 256 MiB。

`plane-state.json.pending_purge` 是单个未闭合清除的审计日记，与前文“待移出排除的文件夹集合”不同。它先记原事件号与计划数，再记删除可能开始，删除返回后先记结果、再追加 Outcome；Intent 后根复验失败也闭合已知失败结果。重开 `sync-purge-residue` 仅补原事件的 Intent／Outcome，不重放删除；删除期间崩溃而结果未落盘时，Outcome schema 2 的计数和 applied 为 null，明确未知。日记未闭合时拒绝平面启动、新轮和重绑定；日记为空时旧 schema 1 文件及校验和不变。

安全配置变更使用显式 `sync-rebind`，维护过程、物理身份限制与确认步骤见部署手册 8.1。范围迁移、整份 owner.json 的 scope_hash 调整与冲突路径排除后推进基线仍是独立未决，不因本轮代码完成而关闭 P6。


### 大批次关停与保全容量（BUG-023／024）

持久保全源沿用七天保留与退休双条件，数量上限4096匹配批准路径预算；单件256MiB、合计512MiB、索引1MiB和唯一接收/Prepared槽不变。临时件和孤儿仍计数，满额拒绝、不清除恢复源换取空间。
执行器在每个64条审计批次与每条动作开始前读取平面关停标志。已Issued动作完成后端调用与真实终态落盘后才退出；未知仍保留Issued。返回Stopping、不发后续动作、不settle或推进基线；原冻结计划、幂等键、outbox在重开后续做。单个后端调用或文件IO本身仍不能强制中断。

冻结动作的源或目标发生变化时，执行器先取得真实新鲜双侧观察。BridgeState 仅对 Intent 已 ACK、从未发出且无回执的预约原子记录 `CancelledBeforeInstall`，不会返回 `IssuedJob`、调用文件写后端或保全陈旧源。持久 `issued` 位与取消票据同事务写入，表示原预约已消费，旧版读取仍为 Known；`bridge-unissued-cancel-<request digest>` 明确标识由持久 owner 取消。已发未知动作绝不走此入口。NoEffect 仍需真实 Outcome/Summary ACK，旧共同基线不推进，下一轮按现状重新规划。
