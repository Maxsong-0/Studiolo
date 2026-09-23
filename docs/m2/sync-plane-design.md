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

## 2 一轮驱动的精确顺序（`sync_plane/driver.rs::run_round`）

| # | 步骤 | 失败即停在哪 | 重开怎么续 |
|---|---|---|---|
| 1 | `recovery::PlaneStateStore::open` → `decide(state, 审批模式, now)` | 状态文件撕裂→`Storage`，本轮不跑 | 下一轮重读；撕裂须管理员 `sync-init` 重建（不自动重置） |
| 2 | `flush_transport_audit`；活动槽存在则 `recover(run_id)`；未闭合→`Transport` | 保留原槽 | 每轮先补审计、再 recover，直到 Closed |
| 3 | `run_headless(InspectConfig)` → `config_output` → `sync_transport_config::verify`：`syncMode`==应处模式、`excludedFolders` 逐项==审批、`fileTypes` 含五类 | 不一致→`ConfigMismatch`，健康降级 | 模式不符走第 5 步；排除项／类型不符只能人改，每轮重核 |
| 4 | `recovery::residue_guard(上次核验集合, 当前集合, 视图根)` | 有残留→`ResidueBlocked`，固定原因「排除项清除前有镜像残留」 | `studiolo-maintain sync-purge-residue <folder>` 清掉后自动放行 |
| 5 | 需要时 `run_headless(SetMode(effective_mode))`，回到 3 复核 | SetMode 非 0 退出→`Transport` | 下一轮重试；`applied_mode` 只在复核通过后写 |
| 6 | `mark_started(seq)`（持久「已启动」）→ `run_headless(RunOnce)` → `mark_reaped(clean)`；非 clean 记退避 | RunOnce 非 0／未静止→本轮停，`not_before=now+5s`（rc=1 连续按 5/10/…/300） | 断电：`started_sequence` 仍在→下一轮强制 `MirrorRemote` 恢复一次 |
| 7 | `OwnerQuiescence::take`（`quiescent()`，窗口 1800 s） | 无静止证明→`Quiescence` | 下一轮重跑 RunOnce |
| 8 | `observe_both`：canonical 经接缝完整观察，镜像经视图完整观察，`approve_union` → `Input` | 任一侧不完整／范围未批→`Observation` | 下一轮重观察 |
| 9 | MirrorRemote 时 `plan` 试算；有冲突→`preresolve_conflicts`（D6）→ 重做第 8 步一次 | 预解决删除越熔断→`Fused`（不解决，健康降级） | 人在界面选边（后续任务） |
| 10 | `bridge.reserve(version, input)`：Fused→`Fused` 停；Ready→`StateJournal`＋`execute(plan, ports, now, deadline)` | 逐动作 Issued 后未知→`Unknown`，本批不闭合 | 下一轮 `retry_pending` 原键核对（不重放） |
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
- Bidirectional（P6）下冲突不自动解决：只保全双方版本到 `ops/bridge-conflicts/` 并在健康节报 `conflict_copies`，等人在界面选边。
- **P6 前必须解决的风险**：整批基线不前进的规划器限制（`settle` 只在无冲突、全成功时推进基线）在 P6 会造成
  「一个未解冲突冻结全库基线」——所有其他路径的共同观察也不会进基线，下一轮仍按旧基线规划。P6 开工前需在
  sync-bridge 加「冲突路径排除后推进基线」的显式入口（另立任务，属 sync-bridge 写域）。
- `_规则/` 在 `sync_scope::paths::hard_excluded` 里是硬排除；D2 拍板的范围里 `_规则/` 本轮**进不了** SyncScope，见 ADR-025 未决。

## 5 持久文件清单（所有者、权限、预算）

| 固定名 | 所有者／权限 | 预算 | 写者 |
|---|---|---|---|
| `<transport_private>/{transport.lock,state.json,state.json.publish}` | owner 0600／根 0700 | 64 KiB | sync-transport `store/publish.rs`（不变） |
| `<plane_root>/plane.lock` | owner 0600 | 4 KiB | `sync_plane/recovery.rs` |
| `<plane_root>/plane-state.json`（+`.new`） | owner 0600 | 64 KiB，版本 1，校验和 | `sync_plane/recovery.rs` |
| `<plane_root>/stage/objects/<sha256 hex>` | owner 0600 | 单对象 256 MiB，合计 512 MiB，64 项（含孤儿） | `sync_plane/mirror/stage.rs` |
| `<plane_root>/stage/index.json`（+`.new`）、`stage/receiving.partial` | owner 0600 | 1 MiB | `sync_plane/mirror/stage.rs` |
| `<plane_root>/bridge/{bridge.sqlite3,bridge.lock}` | owner 0600 | 主库 256 MiB | sync-bridge `sqlite/storage.rs`（不变） |
| `/run/studiolo-sync-launcher/launcher.sock` | root:studiolo 0750 目录，socket 0660 | 帧 16 KiB | launcher `serve.rs` |
| `/run/studiolo-sync-launcher/sessions/<session hex>/{release.fifo,report.fifo,stdout,stderr}` | root 0700 | stdout/stderr 按 RunBudget | launcher `serve.rs` |
| `/etc/studiolo/sync-launcher.json` | root 0600 | 16 KiB | 管理员（provision.sh） |
| `/var/lib/studiolo-sync.img` + mount 单元；`/var/lib/studiolo-owner/mirror` idmapped 视图 mount 单元 | root | 4 GiB | provision.sh |
| `ops/bridge-conflicts/`、`ops/bridge-rollback/`（canonical 内，范围外） | Broker | 受 Broker 限额 | 经接缝，主体 Obsidian |

## 6 健康证据字段（D11，`observability::health::BridgeEvidence`）

节 `sync-bridge`／「双向 Bridge」：`last_batch{sequence,unix_seconds}`、`fused`、`unknown_jobs`、`quiescence_valid`、
`conflict_copies`、`mirror_bytes`、`last_exit`（clean／non_zero／signal／deadline_exceeded／unconfirmed）、
`recovery`（idle／forced_mirror_remote／backoff）、`residue_guard`（clear／blocked）、`mode`。任一 `None` → 未知；
`fused==true`／`unknown_jobs>0`／`residue_guard==blocked`／`last_exit≠clean`／`recovery≠idle` → 降级；`quiescence_valid==false` → 未知。
平面未装配（默认关闭）时本节也是未知：与恢复点、白名单两节同一口径，没启用的能力不显示绿。
`owner-liveness` 加 `sync-transport`（回执＝最近一轮时刻，周期 600 s）。`sync-quota`：`SyncUsage` 取镜像字节，
上限＝`manual_quota.account_limit_bytes`，预警＝七成；detail 固定带「非账号用量」；`manual_quota` 缺→未知。
前端契约 `docs/contracts/health-sync-bridge.md` 由 W5 写，本轮不改 `web/**`。

## 7 守卫登记清单（D15，全部由 W1 收口）

| 位置 | 登记 |
|---|---|
| `architecture_support/policy.rs` | `studiolo-sync-launcher => [sync-transport, core]`；local-server 加 `studiolo-sync-bridge` |
| `architecture_cases/dependencies.rs` | crate 数 24；rustix 归属加 `studiolo-sync-launcher` |
| `architecture_support/sync_launcher.rs` | `serve.rs`／`payload.rs`／`unit.rs` 三文件的精确调用清单（形态见各任务书） |
| `architecture_support/sync_transport.rs` | `linux/client.rs`：`UnixStream::connect(socket)`、`.write_all(&frame)` 各一次 |
| `architecture_support/sync_plane.rs` | `stage.rs`／`recovery.rs`／`mirror.rs` 的固定名锚点＋精确调用清单 |
| `architecture_cases/canonical_owner.rs` | `coordinator/sync_bridge.rs` 的七条允许形态（第 4 节） |

## 8 未决

应为空。三条默认选择记在 ADR-025「未决」：`_规则/` 硬排除、`TerminalSummary` 不带容器身份（`config_output` 依赖 W1 补字段）、
预解决删除的熔断判据在 driver.rs 复写。
