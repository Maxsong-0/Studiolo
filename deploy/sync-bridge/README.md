# 双向 Bridge P4 运行手册：可信 launcher 与 headless 镜像盘

T-132 P1（ADR-025 D2／D7）。本目录是管理员置备物：专用身份、4 GiB loop 镜像盘与两个 mount 单元、
root 系统服务 `studiolo-sync-launcher.service`、固定配置样例、审批文档样例。协议与状态机见
`docs/m2/sync-plane-design-launcher.md`，驱动平面见 `docs/m2/sync-plane-design.md`。
**凭据（Obsidian 账号密码、E2EE 口令）只由用户本人在配对时交互输入，不进任何文件、单元、脚本或本手册。**

## 1 拓扑与固定物

| 身份 | 进程 | 看到镜像的路径 |
|---|---|---|
| owner `maxsong`（uid 1000） | `studiolo-human`（驱动平面 `sync_plane`，每 300 s 一轮） | `/var/lib/studiolo-owner/bridge/mirror`（idmapped bind 视图） |
| root | `studiolo-sync-launcher serve`（私有 socket，`systemd-run` 起 transient unit） | 不碰镜像 |
| `studiolo-sync`（系统账号，无补充组） | 垫片 `studiolo-sync-launcher payload` → `node cli.js sync` | `/var/lib/studiolo-sync/bridge/mirror`（真实根，只进 argv） |

固定物：`/var/lib/studiolo-sync.img`（4 GiB ext4，配额靠镜像大小硬限）挂到 `/var/lib/studiolo-sync`
（`var-lib-studiolo\x2dsync.mount`，`nosuid,nodev,noexec`）；`/var/lib/studiolo-sync/bridge/mirror` 以
`X-mount.idmap` bind 到 `/var/lib/studiolo-owner/bridge/mirror`（`var-lib-studiolo\x2downer-bridge.mount`，
磁盘上 payload 身份的文件显示为 owner，owner 写入落盘归 payload）；socket `/run/studiolo-sync-launcher/launcher.sock`
（目录 0750 root:owner 组，socket 0660，只接受 SO_PEERCRED 为 owner uid/gid 的连接）；
每次运行一个 `/run/studiolo-sync-launcher/sessions/<hex>/{release.fifo,stdout,stderr}`（root 0700；
`RuntimeDirectoryPreserve=restart`：launcher 重启后目录仍在，旧 session 只能被接管为「已关门、永不放行」，显式 stop 才清空）。

垫片拿不到 `LISTEN_FDS` 的裸 fd（工作区禁 unsafe），0600 root 的固定配置对 payload 也不可读，所以
launcher 把启动事实（固定 pins、镜像根、配置根、操作）预写进控制 FIFO 再起 unit（`StandardInput=file:`），
垫片把 `(dev,ino)` 报告帧写在 stdout 文件头部，放行是同一 FIFO 上的一个字节，关门就是 launcher 丢掉写端
（垫片读到 EOF 退出 4，绝不 exec）。

## 2 置备顺序（👤 root，逐步、可重复）

1. **运行时闭包**（一次性，root 所有、只读）：Node v22.23.2 与 obsidian-headless 0.0.14 装到
   `/opt/studiolo/node` 与 `/opt/studiolo/obsidian-headless`（版本与 SHA256 以 `headless-trial/p2-10-install.sh`
   为准，只是装到 `/opt/studiolo` 而不是账号家目录）。记下安装目录总摘要（同脚本「安装目录总摘要」一行）。
2. **出二进制**：`deploy/human/release/package.py … --sync-launcher /ABS/STAGING/studiolo-sync-launcher …`
   （r8 发行目录里是 `bin/studiolo-sync-launcher`，0755）。
3. **置备**：`sudo bash <仓库>/deploy/sync-bridge/provision.sh /ABS/RELEASE/bin/studiolo-sync-launcher`。
   它依次：建 `studiolo-sync`（`useradd --system --user-group`，无家目录、不可登录）→ `fallocate` 4 GiB 镜像并
   `mkfs.ext4` → 渲染安装两个 mount 单元与服务单元（`@OWNER_UID@`、`@PAYLOAD_UID@` 等换成真实数字）→
   安装二进制到 `/usr/local/sbin/studiolo-sync-launcher` → 从 `sync-launcher.example.json` 生成
   `/etc/studiolo/sync-launcher.json`（0600 root；**已存在不覆盖**）→ `daemon-reload`、`enable --now` 三个单元 →
   逐项核验（loop backing_file、idmapped 视图属主、镜像根 0700、socket 0660）并打印。
4. **核对固定配置**：样例里 `runtime.closure_digest` 是 32 个零，`node_path`／`cli_path` 是 `/opt/studiolo` 的默认布局；
   改成第 1 步的实际值后 `sudo systemctl restart studiolo-sync-launcher`。字段见第 3 节。
5. **首次配对**（👤 用户本人，交互输入密码，不写进任何文件）：
   `sudo -u studiolo-sync -H env XDG_CONFIG_HOME=/var/lib/studiolo-sync/config LANG=C.UTF-8 /opt/studiolo/node/bin/node
   /opt/studiolo/obsidian-headless/lib/node_modules/obsidian-headless/cli.js sync-setup --vault <远端 Vault> --path /var/lib/studiolo-sync/bridge/mirror`。
   之后一切运行只经 launcher；`headless_mode` 先保持 `mirror-remote`（引导期只下载）。
6. **审批文档**：从 `approval.example.json` 改成 `/home/maxsong/.config/studiolo-human/sync-transport.json`
   （0600，父目录 0700）；`private_root` 先 `mkdir -m 0700`；`plane_root` 固定为视图里的 `/var/lib/studiolo-owner/bridge/plane`（置备已建，与 `owner_view_root` 同一条 idmapped bind——外 stage 对象要 `renameat` 进镜像，跨挂载必 EXDEV），`owner_view_root` 必须就是第 3 步视图里的 `mirror/`。字段见第 4 节。
7. **human 服务单元**（登记到待重启区，由用户执行）：`ReadWritePaths=` 加 `/var/lib/studiolo-owner/bridge/mirror`、
   `private_root`、`plane_root`；`ExecStart` 参数加 `--sync-transport /home/maxsong/.config/studiolo-human/sync-transport.json`；
   `daemon-reload` 后重启。私有配置 `owner.json` 必须已有 `sync_scope`（第 5 节），否则启动报 `StartupError::SyncPlane`。
8. **初始化平面**：`studiolo-maintain sync-init`（W6 落地）在 `plane_root` 建 `plane-state.json`、`plane.lock`、`bridge/`；
   平面状态文件撕裂时也用它重建，不自动重置。

## 3 固定配置 `/etc/studiolo/sync-launcher.json`（0600 root，≤16 KiB）

| 字段 | 含义 |
|---|---|
| `schema_version` | 固定 1 |
| `owner_uid`／`owner_gid` | 允许连 socket 的 owner 身份（`studiolo-human` 的 uid/gid） |
| `payload_uid`／`payload_gid` | 专用 payload 身份 `studiolo-sync`；不得为 0，不得与 owner 重合 |
| `runtime.{version,node_path,cli_path,closure_digest}` | 固定运行时；`version` 只认 `0.0.14`，摘要是 32 字节数组 |
| `mirror_root`／`payload_config_root` | 真实镜像根与 headless 配置根（`XDG_CONFIG_HOME`），都必须在 `loop_mount` 之下 |
| `loop_image`／`loop_mount` | 固定镜像文件与挂载点；probe 用 `/proc/self/mountinfo` 与 `/sys/dev/block/<maj:min>/loop/backing_file` 核对 |
| `inaccessible` | transient unit 的 `InaccessiblePaths=`（视图根、socket 目录、`/etc/studiolo`；`/home` 由 `ProtectHome=yes` 盖住） |
| `payload_cgroup_prefix` | slice 的完整层级展开：`studiolo-sync.slice` 住在 `/sys/fs/cgroup/studiolo.slice/studiolo-sync.slice`（连字符是层级）；unit 以 `--slice=studiolo-sync.slice` 落进去，实测组必须在此之内 |

## 4 审批文档 `sync-transport.json`（owner 0600，≤8 KiB，`enabled` 必须显式为 true）

| 字段 | 含义 |
|---|---|
| `private_root` | 传输私有根（`transport.lock`、`state.json`），owner 0700 |
| `mirror_root`／`owner_view_root` | 真实镜像根（只进 argv）／owner 读写所经的 idmapped 视图 |
| `plane_root` | 驱动平面私有根：`stage/`、`plane-state.json`、`plane.lock`、`bridge/`（BridgeState） |
| `device_name` | 镜像节点在 Obsidian Sync 里的设备名（核对 headless `deviceName`） |
| `headless_mode` | `mirror-remote`（引导期）或 `bidirectional`（P6）；每轮核对，不符就 `set_mode` |
| `excluded_folders` | headless 排除文件夹的唯一母本（有序、去重）；改动后镜像残留须先清（第 8 节） |
| `manual_quota` | 人工从 Obsidian 账号页抄的 `account_limit_bytes`／`used_bytes_manual`／`recorded_at_unix`；缺则健康节该维度未知 |
| `runtime` | 与固定配置同一套 pins（`closure_digest` 是 64 位小写十六进制） |
| `backend` | `service_unit`＝`studiolo-sync-launcher.service`、`launcher_path`、`payload_cgroup_prefix`、`payload_uid/gid`，全部与固定配置一致 |
| `budget` | 预算：`deadline_seconds`、`stop_grace_seconds`、`stdout/stderr_bytes`、`config_json_bytes`（≤64 KiB）、`private_state_bytes`、`log_bytes`、`memory_bytes`、`swap_bytes`（必须 0）、`pids_max`、`cpu_max_percent` |

## 5 同步范围（私有配置 `owner.json` 的 `sync_scope`）

用户 2026-09-23 拍板「规则视为重点纳入」：范围列 `项目/`、`_规则/`、`_模板/` 三条目录规则；`_规则/` 的
`attachment_types` 含 `txt`（`_规则/归档/` 的 txt）。**`_规则/` 纳入同步范围，但人侧网页保存对 `_规则/` 仍然保护**
（`human_content::rules_directory`）：改规则走策略平面与 T-160 的规则管理界面，不经网页保存。

```json
"sync_scope": {
  "version": 1, "vault_uuid": "<canonical vault_uuid>",
  "link_id": "<32 hex>", "remote_vault": "<32 hex>", "scope_epoch": 1,
  "whole_folders": [
    {"prefix": "项目", "markdown": true, "attachment_types": ["png", "jpeg", "pdf"], "max_bytes": 16777216},
    {"prefix": "_规则", "markdown": true, "attachment_types": ["txt"], "max_bytes": 4194304},
    {"prefix": "_模板", "markdown": true, "attachment_types": [], "max_bytes": 1048576}
  ],
  "whole_files": [], "generated_documents": []
}
```

范围只决定 Bridge 桥接哪些路径；镜像本身跟随远端整库（受 `excluded_folders` 约束）。`ops/bridge-conflicts/`、
`ops/bridge-rollback/` 是 canonical 内的保全目录，不在范围内。

## 6 启动参数与初始化

- `studiolo-local-server … --sync-transport <审批文档>`：默认关闭；不给参数即不装配、行为不变。给了参数但私有配置缺
  `sync_scope`，或审批文档／根身份／launcher 探测任一失败，启动报 `StartupError::SyncPlane` 且不监听（不静默降级）。
- 装配顺序：审批文档 → 借用范围追加审批（根 pins、两两不重叠）→ 固定运行时 → 经 launcher `probe` 核验后端
  （systemd ≥ 257、cgroup v2、loop 配额、专用身份）→ 打开传输运行槽 → 打开 Bridge 状态 → 组装平面。
- `studiolo-maintain sync-init`（W6）：初始化或重建 `plane_root`；`studiolo-maintain sync-purge-residue <folder>`：见第 8 节。

## 7 健康节「双向 Bridge」（`sync-bridge`）

`last_batch{sequence,unix_seconds}`、`fused`（熔断）、`unknown_jobs`（Issued 后未知的动作数）、`quiescence_valid`、
`conflict_copies`、`mirror_bytes`、`last_exit`（clean／non_zero／signal／deadline_exceeded／unconfirmed）、
`recovery`（idle／forced_mirror_remote／backoff）、`residue_guard`（clear／blocked）、`mode`。任一 `None` → 未知；
`fused`、`unknown_jobs>0`、`residue_guard=blocked`、`last_exit≠clean`、`recovery≠idle` → 降级；`quiescence_valid=false` → 未知。
平面未装配时整节未知（没启用的能力不显示绿）。`sync-quota`：镜像字节对 `manual_quota.account_limit_bytes`，七成预警。
`owner-liveness` 多一项 `sync-transport`（最近一轮时刻，周期 600 s）。

## 8 残留清理 `sync-purge-residue`

`excluded_folders` 去掉某文件夹后，镜像里可能仍有它的旧内容；每轮第 4 步 `residue_guard` 发现残留即停在
`ResidueBlocked`（固定原因「排除项清除前有镜像残留」）。管理员确认后跑 `studiolo-maintain sync-purge-residue <folder>`
（经 owner 视图删除该文件夹的镜像残留，走审计），下一轮自动放行。它不改审批文档，也不碰 canonical。

## 9 回滚

- 关平面：审批文档 `enabled` 改 false（或去掉 `--sync-transport`）后重启 human；镜像盘、视图、launcher 都可以留着。
- 停 launcher：`sudo systemctl disable --now studiolo-sync-launcher.service`；transient unit 名固定为
  `studiolo-sync-payload-<sequence>-<run_id 前 8 hex>.service`，`systemctl list-units 'studiolo-sync-payload-*'` 可查残留，
  `systemctl stop`／`reset-failed` 即卸载。
- 换回旧二进制：重新跑 `provision.sh <旧二进制>`（按摘要替换）后 `restart`。
- 镜像盘只在用户明确要求时删除：`disable --now` 两个 mount 单元后再删 `/var/lib/studiolo-sync.img`；配对信息在盘上的
  `config/` 里，删盘等于重新配对。

## 10 置备后自检（👤 root）

```sh
findmnt /var/lib/studiolo-sync /var/lib/studiolo-owner/bridge/mirror        # loop 与 idmapped 两个挂载
stat -c '%U:%G %a %n' /var/lib/studiolo-sync/bridge/mirror /var/lib/studiolo-owner/bridge/mirror
systemctl status studiolo-sync-launcher.service --no-pager
stat -c '%U:%G %a %n' /run/studiolo-sync-launcher /run/studiolo-sync-launcher/launcher.sock
sudo -u maxsong /ABS/RELEASE/bin/studiolo-local-server … --sync-transport …   # 启动即做一次 probe
```
