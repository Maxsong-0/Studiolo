# F4 告警与 F6 启动对时

本目录是候选发布源，尚未安装到生产。选型是 Pushover 主通道、Bark 备用、Healthchecks.io 与 Cloudflare Worker 两套独立外部心跳；接收端是 iPhone。真实手机送达、掉电恢复、购买、密钥注入和生产部署分别留证，不能用测试结果代签。

## 运行边界

- 108 的系统 timer 每分钟读取 169 的私有探针回执；回执必须属于当前 boot 且不超过 120 秒。SSH 只读取不含凭据的七个布尔值，固定命令、不接远端正文。
- 169 的系统 timer 在 `LimitMEMLOCK=infinity` 下查询三个系统单元、对时、内存预算、真实 MCP 握手，并只读人侧的同步状态回执（`docs/contracts/sync-status-receipt.md`），再原子保存回执。MCP 复用私有会话；失效才重新初始化，读取最多 64 KiB、每次八秒。
- 两次相同故障触发，发送成功后冷却 600 秒（换代号就重新算）；A07、A08 是一天，而且每个代号单独记，中间被别的代号打断、或两者互换再回来都不重新算，看到正常才清；时钟回拨直接触发。发送前持久化轮次，再由 `studiolo-alert` 先写 Intent 审计后投递。两路都失败、审计不可用、状态损坏时不能报成功；108 停发自身心跳，让外部检测兜底。
- 两个节点分别向 Healthchecks.io 和 Worker 发空正文。Worker 180 秒没收到就告警；每节点 45 秒发送租约，只有成功才开始 600 秒冷却。超时可能已被供应商接收，跨供应商不承诺恰好一次。
- A01：远端探针不可用；A02：服务或 MCP 不可用；A03：AI 副本过期；A04：内存达到 80% 或无法确认预算；A06：时钟未同步或回拨；A07：Bridge 同步两边没对上满 6 小时（回执读不到或坏了也算）；A08：远端删了文件、主库没开文件操作，同步整批暂停（删除守卫 15 分钟内证明过才算，期间不另报 A07）；A108/A169：对应外部心跳中断。只发代号及可选受登录门禁保护的链接。
- Critical：Pushover priority 2、retry 60、expire 3000；主通道未确认才转 Bark critical/call。Warning：Pushover 普通、Bark passive。回执表示服务接受，不能称手机已响。
- Python 运维进程在读秘密前禁 core、禁同 UID 检视、锁定现有和未来内存，输出固定保护回执；Rust 入口使用现有 `studiolo-process-safety`。不给 SSH/systemctl 子进程传告警密钥。源和凭据均不通过第三方监控上传。

## 候选包与部署预检

在 108 用仓库标准回路构建 `cargo --locked build --release -p studiolo-local-server --bin studiolo-alert`。候选包根保留以下布局，并生成文件 SHA-256 清单；所有源文件必须来自同一已验收提交：

```text
bin/studiolo-alert
deploy/alerts/*.py（排除 test_*.py）
deploy/alerts/systemd/*
scripts/check-agent-replica.py
scripts/check-agent.py
```

代码安装到 root 拥有、不可组写的 `/opt/studiolo-alert/releases/<提交号>/`，`current` 指向这个发布目录。回滚只切回前一包；不删除状态或审计。安装前在两台核实包摘要、Python 版本、`systemctl`、`timedatectl`、SSH 主机指纹、169 三个系统单元的真实名称与 MemoryMax。不自动接受 SSH 主机密钥。

由用户在每台建立以下 maxsong 拥有的私有目录（0700），不要改既有目录权限来掩盖不安全状态：

- `~/.config/studiolo-alert/`：该机的 `.env` 文件；169 另有 `agent-token`（0600），使用已授权的只读类型凭据。
- `~/.local/share/studiolo-alert/` 及 `watch/`、`audit/`（108），`probe/`、`probe-receipt/`（169）。各子目录 0700，自动生成的文件 0600。

108 的 `n108.env` 与 169 的 `n169.env` 由用户经安全输入写入，不能从聊天/日志复制秘密：

| 名称 | 哪台 | 来源 |
|---|---|---|
| `STUDIOLO_HEALTHCHECKS_URL` | 两台，各不相同 | 各自 Healthchecks check 的 HTTPS ping URL |
| `STUDIOLO_WORKER_HEARTBEAT_URL` | 两台 | 已部署 Worker 的 `/heartbeat/n108` 或 `/heartbeat/n169` |
| `STUDIOLO_WORKER_HEARTBEAT_TOKEN` | 两台，各不相同 | 用户生成并注入的对应 Worker secret |
| `STUDIOLO_ALERT_PUSHOVER_TOKEN`、`STUDIOLO_ALERT_PUSHOVER_USER` | 108 | 用户 Pushover 应用与接收人 |
| `STUDIOLO_ALERT_BARK_DEVICE_KEY` | 108 | 用户 iPhone 的 Bark device key |
| `STUDIOLO_ALERT_LINK` | 108，可不设 | HTTPS 登录页，禁止 userinfo/query/fragment |

环境文件必须 0600。先运行不带 `--deliver` 的入口，只应得到 `dry_run_no_delivery`。不要用真实凭据跑测试，不在命令行、屏幕、报告或压缩包中放秘密。

## Worker 与外部检测

在 `worker/` 内用锁文件 `npm ci`，Node >=22.18；`npm test`、`npm run build` 均交 108。该项目使用 `cf` 和 Vite 2 beta，配置是 `cloudflare.config.ts`，无需另加 Wrangler 配置。

生产批准后，通过当前 `cf --help` 核对该锁定版本的资源/secret 命令，创建配置所指 D1 并**只在空新库**执行 `migrations/0001-heartbeats.sql`，核对恰有 n108/n169 两行，再发布构建产物。不得把建表迁移当可重复清库命令。注入 `NODE_108_TOKEN`、`NODE_169_TOKEN`、`PUSHOVER_TOKEN`、`PUSHOVER_USER`、`BARK_DEVICE_KEY` 五个 secret；可选链接在 env 另设 `ALERT_LINK`。两个节点不能共用 token。

配置 Worker 每分钟 scheduled trigger；验收 D1 落点、cron 成功/失败日志及受限 token 后才切定时器。Healthchecks 创建两个独立 check，周期一分钟、宽限两分钟，邮件兜底由 Healthchecks 自身发，不经过 Worker；是否允许通知、邮件接收地址及付费均由用户确认。

## 系统单元与 F6

108 使用 `studiolo-alert-watch.service/.timer`，169 使用 `studiolo-alert-heartbeat.service/.timer`，均为系统单元且 `User=maxsong`。A07、A08 依赖人侧 r20 起写的同步状态回执：先上 r20 再装本包，否则 169 会一直报 A07；两台必须装同一个包（旧包的 5 字段回执在新包看来是 A01）。看门狗状态从这一版起是第 2 版（多记 A07／A08 的发送时刻），第 1 版的状态文件读不进：F4 还没上过生产，没有旧状态要迁。先 `systemd-analyze verify`、现场私有目录/文件预检，再批准安装和启用；不要启动两份同名看门狗。

169 的 agent、human、tunnel 三个单元各自使用 `10-time-sync.conf` drop-in。它只增加启动排序和真实 `NTPSynchronized=yes` 闸门，不假定 chrony/timesyncd；120 秒仍未对时则拒绝启动，由原服务的重启策略处理。`clock_ready` 日志带 boot ID 和等待秒数，只证明对时，不证明服务健康。随后查看同 boot 的探针回执与 108 检查结果。

这会改变生产启动行为，必须先核现场原单元并在维护窗口批准应用。主板电池、BIOS 来电自启、UPS 和真实拔电恢复仍是现场项。没有在本开发会话中实施。

## 逐项验收与回退

1. 假环境：连续失败、冷却、重启保留轮次、坏状态、目录链接、有限输出、进程保护失败关闭、旧 boot/旧回执全部过；Workers 真运行时的空 POST 和跨节点拒绝过。
2. 批准后的真实通道：主路一次；主路故障转备一次；双失败不能记成功；再由用户确认 iPhone 静音/锁屏/专注模式下的实际响铃与确认行为。记录时刻和固定代号，不留原始响应或密钥。
3. 分别停**本候选的**单节点心跳，再两条同时停，检查 Worker 与 Healthchecks 独立触发；恢复后再次中断，确认新情节可告警。不要用停生产应用代替这一步。
4. 维护窗口冷启动：记录断电、开机、对时、三个服务、MCP、隧道、外部心跳各自恢复时间；用真实读数给 RTO。未实际拔电不得勾选。
5. 回退前先停本候选的两个 timer、核对无运行中的 oneshot；切前一包或移回本次 drop-in 的备份、daemon-reload。原生产服务的重启仍要维护授权；保留审计和故障状态。人侧从 r20 退回 r19 时同步状态回执不再更新，约 6 小时后会每天报一次 A07，而同步可能其实正常：退回人侧之前先停 108 的 `studiolo-alert-watch.timer`，或者接受这条误报。

验证记录见 T-190 批次档案。代码检查通过不等于上述生产验收完成。
