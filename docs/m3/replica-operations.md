# M3 自动副本同步的启用与回滚

对应 T-036／T-085，协议见 [资料同步与新鲜度](replica-freshness.md)。本文件是操作步骤，不能作为已经启用的证据。

## 1. 检查与首次同步（Mac）

在仓库根运行：

```bash
bash scripts/sync-agent-vault.sh --dry-run
bash scripts/sync-agent-vault.sh
```

`--dry-run` 只在本地校验白名单并显示摘要，不连接或改动服务器。实际同步使用现有 SSH 主机信任与非交互认证，
目标为 `maxsong@192.168.100.169:/home/maxsong/.local/share/studiolo-agent-replica`。
可通过 `STUDIOLO_VAULT`、`STUDIOLO_HOST`、`STUDIOLO_REMOTE_REPLICA` 显式覆盖，白名单仍固定为 `项目/Studiolo` 和 `_规则`。

只传输这两个目录的可见内容。脚本不会修改本地笔记，也不会覆盖旧 `studiolo-agent-vault` 副本或任何 systemd 服务。
只有接收端验证全部字节与清单成功才发布；失败保留上一完整批次。源机断网或休眠时，服务器能判断副本已经过期，不能知道本地最新笔记内容。

## 2. 更新 MCP 服务（服务器）

> **本节是 T-036 的历史部署步骤，不适用于当前 M4 升级或回滚。**
> 2026-09-19 的 M4 发布已把线上审计库升级到 v2；T-040 旧二进制不能打开它。
> 回退必须有与旧发布配套的 v1 账本备份，不能仅恢复旧单元或旧二进制。
> 交接时升级备份已丢失，尚无经验证的 T-040 干净回滚路径；不得套用本节末尾的历史命令。

> **部署前置（T-065，2026-09-17）：新单元必须先生效，否则服务起不来。**
> `studiolo-agent` 自 T-065 起与另外三个持明文秘密的程序共用秘密驻留面加固，
> Linux 上会 `mlockall` 锁住持有类型 token 与 bearer 的页。`RLIMIT_MEMLOCK` 不足时
> 加固**失败关闭并终止进程**（这是正确行为，不得改成尽力而为），单元会反复启动失败。
> `deploy/agent/studiolo-agent-system.service.in`、`deploy/agent/studiolo-agent.service.in`，
> 以及 `install-t036-system.sh` 与 `upgrade-read-supply.sh.in` 里 `systemd-run` 起的 4401
> 阶段验收临时实例，都已加上 `LimitMEMLOCK=infinity`（`5ff4985`）；
> 升级时**必须连同新单元一起安装**，只换二进制会挂。
> 在 4401 预检时用 `systemctl show -p LimitMEMLOCK <单元>` 确认该值已生效，
> 并在 `journalctl` 里核对启动时那条 `秘密驻留面：…` 记录的四项分类。
> 契约见 [秘密驻留面](../m2/secret-residency.md) §5。

先构建并校验新的 `studiolo-agent` 运行包，执行随包原生测试；打包方法沿用 [服务器部署](server-agent.md)。
托管副本使用系统级 `studiolo-agent-system.service`，进程仍以 `User=maxsong` / `Group=maxsong` 运行。
保留 `ProtectSystem=strict`、`ProtectHome=read-only`、私有库唯一可写、`NoNewPrivileges` 与资源上限，并显式 `PrivateUsers=no`。其参数为：

`--allow` 不在文档里抄列（抄过两次都过时了）：以 `deploy/agent/studiolo-agent-system.service.in` 为准，它与 `scripts/replica_common.py` 的 `ALLOW` 逐项相等，由 `DeployAllowlistTests` 守卫。

必须先由系统管理器使用相同隔离参数，在独立回环端口（例如 4401）启动新程序，验证实际 PID/UID/二进制、initialize、四工具与匿名 401；
确认新鲜度、版本和读取审计后，再安装 `/etc/systemd/system/studiolo-agent.service`，停止并禁用旧用户级 MCP 单元，启用系统级单元。网页和 Tunnel 无需调整。
私有身份库/审计账本沿用，不重复初始化、不重新签发 token。真实凭据不放进聊天、命令参数或日志。

2026-09-10 实测：Ubuntu 26.04 的用户 systemd 在挂载隔离下只映射当前用户，宿主 root 显示为 UID 65534。
因此托管副本会正确拒绝该环境中的未知祖先属主；仅在用户单元写 `PrivateUsers=no` 不能解除这个隐式映射。
不能把 65534 或观测到的根 UID 一律当作可信 root，因为其他未映射宿主用户也会折叠成同一个值。使用系统管理器创建隔离，不修改 Rust 的属主验证。

本次发布的一次性安装器在 `deploy/agent/install-t036-system.sh`；它只接受已校验的 `m3-t036-20260910-replica01` 二进制和系统单元摘要，不是任意版本的通用安装入口。
上传后的执行方式为 `ssh -t maxsong@192.168.100.169 'sudo bash /home/maxsong/studiolo-t036-deploy-system.sh'`。
安装器先在系统管理器的 4401 单元检查实际二进制、UID、用户命名空间及完整供给链，通过后才停旧用户服务并切到系统 4400；错误或中断通过退出守卫恢复旧用户服务，不覆盖既有或被第三方替换的系统单元。

可使用 `python3 -B scripts/check-agent-replica.py <mcp_url> <token文件> --expected ready` 验证握手及四工具回执一致；
停更超过阈值后改用 `--expected stale`。探针只输出已验证的版本与确认时间，不输出凭据或正文。
同步器及 Rust 私有读取要求副本祖先目录不可被组或其他用户写入；既有目录不满足时先核对所有者，再收紧对应写权限，不能放宽接收校验。

回滚时停止并禁用本次系统级单元，再恢复旧用户级单元绑定，执行 `systemctl --user daemon-reload` 和 `systemctl --user enable --now studiolo-agent.service`；两种管理器不能同时占用 4400。
旧单元仍可读取保留的旧副本；该旧副本不会被新同步脚本更新，因此回滚后的内容可能陈旧。

## 3. 生成并启用自动任务（Mac）

使用尚不存在的独立目录；下例父目录须已存在：

```bash
mkdir -p "$HOME/.local/share/studiolo-agent-sync"
python3 scripts/prepare-agent-sync.py \
  --output "$HOME/.local/share/studiolo-agent-sync/t036"
plutil -lint "$HOME/.local/share/studiolo-agent-sync/t036/me.studiolo.agent-sync.plist"
```

此命令固定本次同步代码、解释器和配置。可检查生成的 plist，准备命令本身不联网、不安装任务。
如使用自定义源、主机或远端根，把同样的值传给 `--vault`、`--host`、`--remote-root`；不在环境中传密码。
首次同步及服务器切换验收成功后，安装并启动：

```bash
mkdir -p "$HOME/Library/LaunchAgents" &&
ln -s "$HOME/.local/share/studiolo-agent-sync/t036/me.studiolo.agent-sync.plist" \
  "$HOME/Library/LaunchAgents/me.studiolo.agent-sync.plist" &&
launchctl bootstrap "gui/$(id -u)" "$HOME/Library/LaunchAgents/me.studiolo.agent-sync.plist"
```

`ln` 遇到既有配置会停止，不覆盖其他安装。任务在用户登录时启动，每 60 秒运行一次；launchd 不重叠运行同一任务，
同一配置目录中的同步进程另有本地锁，接收端也独占锁定目标，防止手动同步与自动同步交错发布。网络失败返回非零，下一轮重试，单次传输限时 120 秒。
任务依赖 Mac 用户登录、Vault 可读及现有 SSH 非交互认证；休眠期间不会同步。超过 300 秒未确认的资料由 MCP 标为陈旧。

检查：

```bash
launchctl print "gui/$(id -u)/me.studiolo.agent-sync"
```

运行目录的 `state/` 保存互斥锁，`logs/` 保存不含正文和凭据的同步结果与错误。后台认证失败先在用户终端检查 SSH 配置；不要把密码加入任务。
更新同步器时生成新的独立目录，停止旧任务并更换 plist 链接后再启用；不会因仓库切换分支自动换代码。

### T-085：修复 macOS 后台 Python 的局域网权限失败

`sync-agent-vault.sh` 由 **bash 直接启动 SSH**；Python 分阶段完成安全捕获、私有归档和封闭回执校验。
整批同步使用 bash 持有的继承文件描述符锁，捕获结束后直到回执核验完成仍拒绝并发批次。
截止分两段：取锁＋捕获 120 秒；SSH＋回执按本次实际发送的字节数给（`120 + ceil(字节/1 MiB)` 秒，见 `replica_archive.transfer_deadline`，T-131）。退出或中断会回收 SSH 和临时目录。归档与响应仅写到 0700 私有临时目录的 0600 文件。
SSH 失败只报告数字 `ssh_exit_code`；超时报告 `transfer_timeout`，不会把远端 stderr 或资料正文写入日志。
响应有文件大小上限，超过 8192 字节、摘要不匹配、旧确认时间或额外字段均拒绝。

### T-131：增量同步（确认／增量／整份）

每轮先与 `state/base-<key>.json` 比较：这份基线是上一轮被服务器确认过的清单原文（0600，原子替换）。
- **确认**：清单与基线相同时，只发清单和请求，约一个 10 KiB 的 tar 记录。服务器核对现代次完好后只刷新回执时间，不写正文。
- **增量**：只发全部目录和与基线不同的文件。其余文件由服务器从现代次本机复制，然后与整份同样全量核对，再原子发布。
- **整份**：没有基线（首次运行、基线已清、白名单变了）时，与旧行为相同。

服务器回 `base_mismatch`（现代次不是这份基线，或现代次已损坏）时，同步报 `{"status":"failed","last_error":"base_mismatch"}` 并清掉基线，下一轮自动整份推送。现代次损坏时服务器另记 `validation_failed`，agent 状态会如实显示。同步结果 JSON 多两个字段：`mode`（full／delta／confirm）和 `sent_bytes`。需要强制整份推送时，删除 `state/base-*.json` 即可。

遇到 `launchd` 连续 `transfer_failed`、但交互 shell 成功时，检查实际运行的冻结包是否包含这项修复。
不要仅以手工推送成功判定恢复：新包必须由 launchd 定时触发、连续至少两轮退出码 0，
且服务器 `check-agent-replica.py --expected ready` 通过。替换前保留旧 plist 目标和旧运行目录；新任务失败则恢复它们。

停止自动同步：

```bash
launchctl bootout "gui/$(id -u)/me.studiolo.agent-sync"
```

随后只移除本次安装的 `~/Library/LaunchAgents/me.studiolo.agent-sync.plist` 链接，避免下次登录重新启动；保留运行目录和服务器副本便于排查。

## 4. 验收口径

- 本地修改一篇白名单笔记，下一轮同步成功后，MCP 正文、搜索和路牌的版本一致且内容更新。
- 连续两次请求保持同一源摘要时，成功确认时间可更新；仅后台扫描不能续鲜。
- 停止自动任务超过阈值，工具明确显示陈旧；手动成功同步后恢复。
- 同步最近失败与“超过确认期限”分开显示；远端控制文件无效、文件摘要不符或扫描不完整时拒绝内容。
- 链接/权限/损坏等负向验证使用隔离夹具，不破坏线上 Vault。
