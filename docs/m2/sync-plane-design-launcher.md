# T-132 P1：可信 launcher 协议（ADR-025 D2／D3）

配套 [`sync-plane-design.md`](sync-plane-design.md) 第 3 节。DTO 母本：`crates/sync-transport/src/linux/protocol.rs`
与 `protocol/response.rs`；帧＝4 字节大端长度 + JSON 对象（≤16 KiB，`ipc::encode_frame`）；顶层键闭集 `ipc::ALLOWED_KEYS`。
**可执行路径、镜像路径、argv 正文、token、env 永远不进帧**：argv 只送 `argv_digest`（`service::argv_digest`），
launcher 用 `/etc/studiolo/sync-launcher.json` 里的固定 pins 重算比对。

## 1 请求（owner → launcher）

| `kind` | 字段 | 语义 |
|---|---|---|
| `probe` | `nonce[32]` | 探测前置；不建 unit |
| `boot_id` | — | 当前 boot |
| `launch` | `run_id`、`sequence`、`operation`（`inspect_config`/`run_once`/`set_mode`）、`mode?`、`nonce[32]`、`argv_digest[32]`、`budget{RunBudget}`、`payload_uid`、`payload_gid` | 建 transient unit，payload 停在放行 FIFO 前 |
| `release` | `ContainerFrame` | 一次性放行；第二次 → `busy` |
| `running` | `ContainerFrame` | 垫片是否已 exec |
| `gate` | `ContainerFrame` | 关闭 spawn 入口（作废本 session 的放行） |
| `events` | `ContainerFrame` | 读 unit cgroup 的 `cgroup.events` |
| `drain` | `ContainerFrame` | stdout/stderr 已排空字节与超限位 |
| `stop` | `container{ContainerFrame}`、`grace_seconds` | `systemctl kill --signal=TERM`，宽限后 `KillMode=control-group` 强停 |
| `reap` | `ContainerFrame` | `ExecMainStatus`/`ExecMainCode`（`RemainAfterExit=yes` 保证退出后仍可读） |
| `match` | `ContainerFrame` | 重开身份核对 |
| `output` | `ContainerFrame` | reap 后 stdout 正文（UTF-8，≤`config_json_bytes`） |

`ContainerFrame`＝`run_id`、`boot_id`、`unit_name`、`invocation_id`、`payload_cgroup`、`cgroup_identity(dev,ino)`、`launcher_session`；
launcher 逐字段与 session 记录比对，任一不同 → `identity_mismatch`。

## 2 应答（launcher → owner）

`refused{verdict}` 可回任何请求；`verdict` 闭集与 `ContainmentGap` 映射（`Verdict::gap`）：
`protocol`/`argv_mismatch`→`ProtocolViolation`，`peer`→`PeerRejected`，`identity_mismatch`/`busy`→`IdentityMismatch`，
`manager_unavailable`→`ManagerUnavailable`，`budget`→`BudgetUnsupported`，`not_provisioned`→`TrustedLauncherNotProvisioned`。

| `kind` | 字段 | 来源（全部实测） |
|---|---|---|
| `probe` | `nonce`、`protocol_version`、`systemd_version`、`cgroup_v2`、`loop_quota`、`dedicated_identity`、`payload_uid`、`payload_gid`、`boot_id` | `systemctl --version`、`/sys/fs/cgroup/cgroup.controllers` 存在、mountinfo+`/sys/block/loopN/loop/backing_file`==配置 `loop_image`、`getpwnam`/`getgrouplist` |
| `boot_id` | `boot_id` | `/proc/sys/kernel/random/boot_id` |
| `launch` | `nonce`、`container{ContainerFrame}`、`limits{memory_max,memory_swap_max,pids_max,cpu_max}`、`isolation{7 bool}`、`diagnostics{pid,start_time}` | `systemctl show -p InvocationID,ControlGroup,MainPID,ExecMainStartTimestampMonotonic`；`stat(/sys/fs/cgroup<ControlGroup>)`；读 `memory.max` 等；七项见 §4 |
| `release`/`gate`/`stop` | 空 | — |
| `running` | `running` | 垫片 exec 后 `MainPID` 的 `/proc/<pid>/exe` 是固定 node |
| `events` | `populated`、`frozen` | `cgroup.events` |
| `drain` | `stdout_bytes`、`stderr_bytes`、`output_over_budget` | session 目录 stdout/stderr 文件大小；超 RunBudget 即置位并 `stop` |
| `reap` | `exit_observed`、`exit_code?`、`exit_signal?` | `ExecMainCode`（exited/killed）+`ExecMainStatus`；`Result` 为 `exit-code`/`signal` 时才 `exit_observed=true` |
| `match` | `identity` | `exact`/`unit_missing`/`unit_replaced`/`cgroup_reused`/`boot_changed` |
| `output` | `output` | session stdout 文件；非 UTF-8 或超预算 → `refused{budget}` |

## 3 session 状态机（`crates/sync-launcher/src/session.rs`）

`Barrier`（unit 已起、垫片已报 (dev,ino)、未放行）→ `Released`（写放行 FIFO 一字节）→ `Gated`（不再放行）→ `Reaped` → `Retired`（`systemctl stop`，删 session 目录）。
一次只有一个非 Retired session；`launch` 到来时旧 session 须已 `Reaped` 且 `populated=0` 才卸载替换，否则 `busy`。
`release` 只在 `Barrier` 接受一次；`gate` 在 `Barrier`/`Released` 都可，之后任何 `release` → `busy`。
超时：socket 单帧读写 5 s；`systemd-run` 等待 unit 进入 `active` 10 s；`systemctl stop` 等待 `grace_seconds+10` s；垫片报告 FIFO 等待 10 s。

## 4 transient unit 与七项隔离的实测来源

`systemd-run --unit=studiolo-sync-payload-<sequence>-<run_id 前 8 hex>.service --uid=studiolo-sync --gid=studiolo-sync
--service-type=exec -p RemainAfterExit=yes -p KillMode=control-group -p PrivateMounts=yes -p PrivatePIDs=yes -p ProtectSystem=strict
-p ProtectHome=yes -p ProtectControlGroups=yes -p PrivateTmp=yes -p PrivateDevices=yes -p NoNewPrivileges=yes
-p CapabilityBoundingSet= -p RestrictAddressFamilies="AF_INET AF_INET6" -p ReadWritePaths="<mirror_root> <payload_config_root>"
-p InaccessiblePaths="<config.inaccessible 逐项>" -p MemoryMax=<memory_bytes> -p MemorySwapMax=0 -p TasksMax=<pids_max> -p CPUQuota=<cpu_max_percent>%
-p OpenFile=<sessions>/<hex>/release.fifo:release:read -p OpenFile=<sessions>/<hex>/report.fifo:report -p StandardOutput=truncate:<sessions>/<hex>/stdout
-p StandardError=truncate:<sessions>/<hex>/stderr -- /usr/local/sbin/studiolo-sync-launcher payload`（argv 由垫片按 `LaunchFrame.operation/mode` 从固定 pins 构造，不过 IPC）。

| `IsolationFrame` 项 | 实测 |
|---|---|
| `mount_namespace` | `systemctl show -p PrivateMounts,ProtectSystem,ProtectHome,InaccessiblePaths` 逐项等于请求值 |
| `pid_namespace` | `PrivatePIDs=yes` 回读（systemd ≥ 258 才有此属性；`probe.systemd_version` < 258 → `not_provisioned`） |
| `dedicated_identity` | `User`/`Group` 回读 == 配置 payload 身份，且 `SupplementaryGroups` 为空；垫片报回 `uid/gid` 相同 |
| `dropped_capabilities` | `CapabilityBoundingSet` 回读为空 |
| `payload_cannot_write_cgroup` | `ProtectControlGroups=yes` 且 `RestrictAddressFamilies` 不含 `AF_UNIX` |
| `final_view_identity_checked` | 垫片 `PayloadReport` 的四个 `(dev,ino)` 与 launcher 自己 stat 的一致 |
| `disk_quota_enforced` | 镜像根 `statfs` 的设备是 loop，且 `/sys/block/loopN/loop/backing_file` == 配置 `loop_image` |

任一项不成立 → `launch` 应答里对应 bool 为 false（owner 的 `isolation_sufficient` 会拒绝并 `ExitUnconfirmed`）；launcher 不自行 `stop`，等 owner 的 `stop`。

## 5 垫片（`studiolo-sync-launcher payload`）

1. 从 `LISTEN_FDNAMES`/`LISTEN_FDS` 取名为 `release`、`report` 的两个 fd；取不到即退出码 3。
2. `stat` 四个对象（`mirror_root`、`payload_config_root`、`node_path`、`cli_path`，路径从 `/etc/studiolo/sync-launcher.json` 读——它对 payload 只读可见），
   把 `PayloadReport{uid,gid,四个 (dev,ino)}` 一帧写入 report fd。
3. 阻塞读 release fd 一字节；读到 EOF（launcher 作废 session）即退出码 4，不 exec。
4. `env_clear`，只加 `XDG_CONFIG_HOME=<payload_config_root 父目录>`、`LANG=C.UTF-8`、`TZ=UTC`（`service::environment_acceptable` 认可），
   `exec` 固定 argv（`service::build_argv` 同一函数）。exec 失败退出码 5。
