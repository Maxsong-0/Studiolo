---
id: ADR-025
日期: 2026-09-23
状态: 已采纳
---

> **快照**：同步自 vault `项目/Studiolo/03-记忆/ADR-025-双向Bridge P1装配架构.md`（2026-09-28）。权威以 vault 为准，本文可能滞后；供读不到 vault 的会话（如 Cursor Cloud）使用，见 [README](README.md)。

# ADR-025 双向 Bridge P1 装配架构

## 背景

[2026-09-22-T-132-双向Bridge上线方案](2026-09-22-T-132-双向Bridge上线方案.md) 核实：T-048 的 Bridge 执行器是纯顺序核，全仓没有装配它；OwnedRun 的 Linux 后端在所有平台都报缺口（`verify_backend` 永不 `Ok`）；健康页看不到 Bridge。P2 试跑（「2026-09-22-T-132-P2-试跑记录」（vault，未同步） 步骤 5）给出九条硬约束：默认配置不能用、镜像只在 headless 退出后读、引导期 `mirror-remote`、双向模式必须配非正常退出恢复协议、强杀后等 ≥5 s、排除项清除前先删残留、删除会传到镜像、附件冲突 headless 不保全等。用户 2026-09-23 拍板 Q1～Q4（脚本进仓库、P3 走 NAS、P1 走 workflow ≥5 个 Fable 5.1 代理、冲突策略已改）。P1 代码阶段需要一次把「谁在哪个进程、以什么特权、按什么顺序、怎么恢复」定死，否则 6 个并行代理各自发明一套。

## 决定

按 15 条冻结决定装配（详见仓库 `docs/m2/sync-plane-design.md` 与 `sync-plane-design-launcher.md`）：

- **D1 进程拓扑**：Bridge 驱动循环装进 `studiolo-human`（local-server）作新平面 `sync_plane/**`，组合根 `startup/sync_input.rs`；默认关闭（无 `--sync-transport` 即不装配）。
- **D2 可信 launcher**：新 crate `studiolo-sync-launcher`，`serve`（root 系统服务，私有 socket 只认 owner uid）＋`payload`（垫片：stat 报 (dev,ino)、读放行 FIFO、`env_clear` 后 exec）。用固定路径 `systemd-run` 起 transient service，属性闭集写死；`IsolationReport` 七项全部由 launcher 实测（`systemctl show` 回读＋垫片报告＋statfs 核 loop），不许凭「已请求」填 true。
- **D3 owner 侧 Linux 后端**：`verify_backend` 经 launcher `probe` 拿事实；`LinuxContainment` 每个方法都是 IPC；新增 IO 文件只有 `linux/client.rs`；载荷字段闭集 `ipc::ALLOWED_KEYS`（44 键），可执行、路径、argv 正文、凭据永不过 IPC，argv 只送摘要。
- **D4 `RunOperation::SetMode(SyncMode)`**：`SyncMode { MirrorRemote, Bidirectional }`，封闭 argv `sync-config --path <mirror> --mode <…> --json`；不开放其它配置修改。
- **D5 引导期语义**：规划与执行始终双向；安全全靠 headless `mirror-remote`；MirrorRemote 下每条出站动作前把 canonical 当前版本保全到 `ops/bridge-rollback/`。P6 只做 `SetMode(Bidirectional)`＋ADR-010 权威切换，没有第二套「只摄入」代码。
- **D6 冲突**：MirrorRemote 下冲突在下一轮规划前按「镜像为准」预解决（保全到 `ops/bridge-conflicts/`，CAS 到镜像版本，缺失则条件删除且受熔断）；Bidirectional 下不自动解决，只保全双方并报健康。首次对齐即此普通情形。
- **D7 镜像视图**：owner 经 idmapped bind 视图读写；`/var/lib/studiolo-sync` 整体在 4 GiB ext4 loop 镜像上，这就是 `disk_quota_enforced` 的依据。审批文档新增 `owner_view_root`（Rust 访问器 `mirror_view_root`）。
- **D8 搬运与保全**：外 stage `<plane_root>/stage/objects/<sha256>`＋`index.json`（7 天保留、孤儿回收）；canonical 侧 CreateOnly／Replace／Delete 全走 Broker 条件写，主体「外部：Obsidian」，回执用真实 revision；镜像安装只在静止窗口内。
- **D9 恢复协议与退避**：`<plane_root>/plane-state.json`；未干净回收→先 `SetMode(MirrorRemote)`＋一次 `RunOnce`；非 `Code(0)` 后至少等 5 s；rc=1 连续按 5/10/20/40/80/160/300 s。
- **D10 排除项守卫**：审批 `excluded_folders` 是唯一母本；每轮 `InspectConfig` 核模式／排除项／五类；已移出集合的文件夹在镜像仍有残留→拒绝 RunOnce，固定原因「排除项清除前有镜像残留」，`studiolo-maintain sync-purge-residue <folder>` 清。
- **D11 健康**：第六节 `sync-bridge`／「双向 Bridge」；`owner-liveness` 接真实回执；`sync-quota` 用镜像字节数并标「非账号用量」，`manual_quota` 七成预警；平面未装配（默认关闭）时第六节为「未知」，与恢复点、白名单同一口径（没启用的能力不显示绿），判定已在地基实现（`observability/health/bridge.rs`）；本轮不改 `web/**`，契约写 `docs/contracts/health-sync-bridge.md`。
- **D12** SyncScope 附件闭集加 `txt`、`json`。**D13** `studiolo-maintain sync-init`／`sync-purge-residue`。**D14** `deploy/sync-bridge/`（README、provision.sh、单元模板、审批样例、headless-trial 六份脚本）＋打包清单。**D15** 守卫登记全部归 W1。

A 轮地基另定了三条实现级细节：`ApprovedTransport: Sync`、`StatePublisher: Send`（owner 要进跨线程 `Mutex`）；审批文档新增 `plane_root`、`device_name`、`headless_mode`、`excluded_folders`、`manual_quota`；`RootPins` 加 `mirror_view_root`（transport 持久 schema 仍为 v1——生产从未初始化过该存储）。

## 备选方案与放弃原因

- 独立 `studiolo-sync` 进程跑驱动循环 → 放弃：Broker 单写者在 `studiolo-human` 里，跨进程再开一条写路径违反 ADR-010。
- owner 自己起 `systemd-run --user`／直接读 cgroupfs → 放弃：uid 1000 拿不到专用身份、mount/PID 隔离与 loop quota 证据，只能靠 root 服务实测。
- 引导期另写一套「只摄入」执行器 → 放弃：两套实现并存；`mirror-remote` 已把出站变成无害回滚，三方基线自然收敛。
- 把 launcher DTO 定义在 launcher crate → 放弃：sync-transport 的 client 也要用，依赖方向只能是 launcher → transport。

## 后果

- 合入 main 不改生产行为（默认关闭）；给了审批文档但 launcher 未置备 → 启动拒绝并报 `SyncPlane`，不静默降级。
- 新私有根与 root 服务要管理员置备（provision.sh，👤）；owner 与 payload 身份严格分离，镜像对 owner 只经视图可见。
- **P6 前风险**：规划器「整批不前进基线」在 P6 会让一个未解冲突冻结全库基线，须在 sync-bridge 加冲突路径排除入口（另立任务）。
- 与方案 P4 措辞的差异：方案写「引导期 create-only、不授权删除」，本决定改为「引导期双向规划、镜像为准预解决、删除仍受熔断且进墓碑」——安全来自 `mirror-remote` 与熔断，而不是关掉方向；主进程已向用户说明。

## 未决（默认选择）

1. `_规则/` 在 `sync_scope::paths::hard_excluded` 是硬排除，D2 拍板的范围本轮进不了 SyncScope → 默认：P1 不改硬排除表，范围先按 `项目/`＋`_模板/`，`_规则/` 由用户决定是否改 ADR-010 保留文档规则后另立任务。
2. `TerminalSummary` 不带容器身份，`config_output` 取 InspectConfig 输出要 W1 在闭合摘要里加 `container: Option<ContainerIdentity>`（schema 仍 v1）→ 默认：W1 加字段。
3. 预解决删除的熔断判据在 `sync_plane/driver.rs` 复写（sync-bridge 不在写域）→ 默认：复写＋等价契约测试；P6 前合回 sync-bridge。
4. 驱动间隔固定 300 s、静止窗口 1800 s、launcher 各超时（5/10 s）不进审批文档 → 默认：常量，P5 试跑后再决定是否可配。

## 2026-09-23 补充（B 轮落地后主进程 C 轮核出的四处实现级修订）

1. **launcher 屏障改为 stdin 控制 FIFO ＋ stdout 头部报告**（W2）：`forbid(unsafe_code)` 下垫片拿不到 `LISTEN_FDS` 的裸 fd，`OpenFile=`／`report.fifo` 弃用；launcher 起 unit 前把 `LaunchFacts` 帧预写进 `release.fifo`，垫片从 stdin 读事实帧、把 `PayloadReport` 写在 stdout 头部、再阻塞读一字节放行。`MIN_SYSTEMD_VERSION=257`（`PrivatePIDs=` 自 257 起）；`XDG_CONFIG_HOME` 直接等于 `payload_config_root`。设计文档已同步。
2. **外 stage 与镜像视图必须同一 vfsmount**（W4 核出）：`install` 是 `renameat` 进视图，跨挂载必 EXDEV。置备改为把 `bridge/{mirror,plane}` 作为一个父目录 idmapped bind 到 `/var/lib/studiolo-owner/bridge`，审批 `plane_root=/var/lib/studiolo-owner/bridge/plane`；`mirror::install` 安装前用 `statx(STATX_MNT_ID)` 核同挂载，不同即 `Storage`；payload 侧由 launcher `InaccessiblePaths=` 藏起 `plane/`。
3. **健康第六节加第十一个字段 `config_matches`**（W5 核出）：`ConfigMismatch` 原本只能落未知，现按设计文档第 2 节第 3 步降级，固定文案「headless 配置与审批文档不一致，本轮未同步」，判定顺序插在排除项残留之后。`冲突副本 n` 取本进程启动以来的累计数（`ops/` 属硬排除，拿不到目录总数；真实计数另立任务）。
4. **Bridge 侧 `FileVersion::revision`**（W3）：`Revision` 无字节访问器，取 `sha256(revision.to_hex())`，原 `Revision` 记在 `driver/evidence.rs` 的 `RevisionTable` 按摘要还原，供条件写／删除前置。另：`_规则/` 自本日起进同步范围（`human_content::reserved_for_agents` 拆出母本，人侧网页保存仍保护，见 T-160）。

## 2026-09-26 补充（P1 遗留修复，取代上节第 4 条）

1. **Bridge 证据直接承载 `Revision` 原值**：`studiolo_core::Revision` 本就是 32 字节摘要（此前误以为无字节访问器），core 加 `from_bytes`／`to_bytes`，
   `FileVersion::revision = revision.to_bytes()`（`driver/evidence.rs::revision_digest`／`revision_from_digest` 互逆）。`RevisionTable` 与
   `reconcile/canonical.rs` 的「接缝有界读还原前置」整段删除：条件写／删除的前置在任何一轮都能按证据还原，已安装的 canonical Replace／Delete
   回执丢失后同样按原键核对（原「P4 前必修」关闭）。`Revision` 是比较用的修订值不是能力，`Deserialize` 本就接受任意字节，字节构造不放宽边界。
2. **每一处镜像读写都经同一张票据复验**：`RoundContext` 带第 7 步的 `QuiescentMirror`，新 `quiescence::verify(plane, ticket)` 是唯一复验入口；
   搬运读 `read_mirror_bytes`、镜像观察 `observe_mirror`、核对读 `reconcile/mirror.rs`、安装 `apply_mirror`、排除项守卫（任务书 A）都先过它，
   作废即 `Quiescence`。流式写的读取发生在接缝之内，只能在交出来源前复验。执行器批内对镜像侧 Delete 的删除前保全读由批首取票与逐动作
   recheck 覆盖，不单独复验（2026-09-28 评审 P3-3 更正「每一处」的说法）。
3. **还没碰接缝的失败记 `NotAttempted`**：`write_from_mirror` 读镜像失败（票据作废、读不出、与冻结版本不符）不再记 `Unknown`——动作确定未安装，
   下一轮重新规划（R1 已知缺口关闭）。
4. **Bidirectional 冲突保全**（「2026-09-26-T-132-P1-遗留-任务书-A」（vault，未同步））：有冲突（含熔断冻结的批）先保全双方现存版本到 `ops/bridge-conflicts/`
   （canonical 版本设备名 `studiolo-canonical`，镜像版本用审批 `device_name`），再照常执行。冲突身份＝`sha256(域‖link_id‖路径‖两侧 sha256)`；
   首次保全戳先写进 `plane-state.json` 的 `conflict_stamps`、再写副本，幂等键 `conflict/<link>/<身份>/<戳>/{canonical,mirror}`——跨轮续写同一副本，
   冲突消失后重现另存一份。上限 512 条，按路径序。
5. **排除项安全顺序**：驱动记下 headless 实际排除集合（`headless_excluded`），审批已去掉、headless 仍排除的为 `pending_purge`；守卫在 InspectConfig
   刚闭合的票据下读视图；残留已清但仍有 `pending_purge` 即 `ConfigMismatch`；`sync-purge-residue` 只在 headless 仍排除时动手，并写 `sync.purge-residue`
   Intent／Outcome 审计。`plane-state.json` 追加字段只在非默认时带标签字节进校验和，版本仍为 1，旧文件照常解码。
6. **关停停点**：关停置标志，正在跑的 headless 被 `request_stop(OwnerRequested)` 停下并回收落盘，各安全停点返回 `Stopping`；执行段不打断。
   健康退出分类加 `OwnerStopped`。
7. **冲突副本计数**：健康第六节「冲突副本」＝最近一次完整观察里两侧命名为冲突副本的路径数（`naming::parse_copy`，两种设备名）＋本进程启动以来 Bridge 保全的冲突数。
8. **审批 `device_name` 收紧**为 ≤64 字符、不含 `(`／`)`／`/`／控制字符：它要原样进冲突副本名，在审批层拒绝，而不是让保全每轮在命名处失败。
9. **批准配置摘要只覆盖传输安全字段**（用户 2026-09-27 同意 (a)）：`host-config::sync_transport::document::digest` 按固定顺序编码四个根、runtime、backend、budget，
   域 `studiolo.sync-transport.approval.v2`；`device_name`／`headless_mode`／`excluded_folders`／`manual_quota` 不进摘要，改后重启即生效（前三项每轮 InspectConfig 核对）。
   传输字段改动的 `sync-rebind` 与 `scope_hash` 绑整份 owner.json 的同类问题另立任务。

## 2026-09-28 补充（遗留评审后的修订）

1. **镜像源变了，核对只能保持未知**：`write_from_mirror` 的镜像源读不出（票据作废、文件变了或没了）单独分类；首次执行记确定未安装，重开核对一律未知——
   内联写的 Broker 指纹含正文，源变了就重建不出首次请求，原键下可能早已安装。让核对不依赖镜像当前内容（外 stage 留存入站源）另立任务。
2. **冲突键与副本路径一一对应**：MirrorRemote 预解决与 Bidirectional 保全共用 `conflict_stamps`；键＝`conflict/<link>/<冲突身份>/<首次戳>/…`，
   冲突身份含两侧版本摘要，镜像副本键另带设备名摘要；每轮规划后按本轮冲突回收旧戳。
3. **副本名按字节封顶**：末段超过 255 字节按字符边界截短并附原名摘要；审批 `device_name` 至多 64 字节、不许与 `studiolo-canonical` 大小写不敏感地相同。
4. **被停下的运行不算干净**：任何停止原因（owner／超时／输出超限）都让 `clean=false`，即便 rc=0（headless 对 SIGTERM 以 0 退出），下一轮走 D9。
5. **`plane.lock` 由租约守卫持有**：拿到锁即交给 `Lease`，析构（含 open 中途失败）显式 `flock(Unlock)`——并发 fork 在 exec 前继承的描述符不再延长租约（main CI 偶发红的根因）。
