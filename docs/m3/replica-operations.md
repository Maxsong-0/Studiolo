# M3 自动副本同步的启用与回滚

对应 T-036，协议见 [资料同步与新鲜度](replica-freshness.md)。本文件是操作步骤，不能作为已经启用的证据。

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

先构建并校验新的 `studiolo-agent` 运行包，执行随包原生测试；打包方法沿用 [服务器部署](server-agent.md)。
新包单元已经使用：

```text
serve --replica-root /home/maxsong/.local/share/studiolo-agent-replica --replica-max-age-secs 300 --allow 项目/Studiolo --allow _规则 --data /home/maxsong/.local/share/studiolo-agent --port 4400
```

第一次切换前，在独立回环端口（例如 4401）启动新程序，使用既有类型身份验证 initialize、四工具与匿名 401；
确认新鲜度、版本和读取审计后，再备份原 `studiolo-agent.service` 的文件或链接并更换单元。只重启 MCP 服务，网页和 Tunnel 无需调整。
私有身份库/审计账本沿用，不重复初始化、不重新签发 token。真实凭据不放进聊天、命令参数或日志。

可使用 `python3 -B scripts/check-agent-replica.py <mcp_url> <token文件> --expected ready` 验证握手及四工具回执一致；
停更超过阈值后改用 `--expected stale`。探针只输出已验证的版本与确认时间，不输出凭据或正文。
同步器及 Rust 私有读取要求副本祖先目录不可被组或其他用户写入；既有目录不满足时先核对所有者，再收紧对应写权限，不能放宽接收校验。

回滚时先恢复备份的单元绑定，再执行 `systemctl --user daemon-reload` 和 `systemctl --user restart studiolo-agent.service`。
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
