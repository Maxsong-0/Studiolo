# Obsidian Headless 0.0.14：SyncTransport 执行契约核查

核查日期：2026-09-13。状态：官方公开资料和精确发布包的静态核查；建议部分待主线冻结。未安装、运行 Headless、登录、读取凭证或同步真实 Vault；未改仓库或运行工具链。只下载官方元数据及包，在内存验证归档摘要并读取三个文本成员，未执行发布代码或 native addon。

2026-09-13补充只读平台核验：已指定服务器 `maxsong@192.168.100.169` 实际UID1000，
`systemctl --version` 为 systemd259（259.5-0ubuntu3.4），具备cgroup v2；PATH无Node、ob、restic或Docker。
尚无受控Headless进程owner、专用隔离运行环境或真实Sync接入，这些条件不能从版本查询推定已完成。
退出证明须核对受控组及全部后代；内核`cgroup.events`的`populated=0`表示整棵组没有存活进程，
但仍须固定组身份并关闭重新派生入口。`KillMode=control-group`规定停止时处理整组，服务状态或单PID不能替代实际空组证明。
依据：[内核cgroup v2](https://cdn.kernel.org/doc/html/latest/admin-guide/cgroup-v2.html#un-populated-notification)、
[systemd官方停止规则](https://github.com/systemd/systemd/blob/main/man/systemd.kill.xml)。这些是下一实现约束，尚未动态验收。
持久运行槽、受控启动和静止能力的实施候选见 `owned-run-design.md`；尚未实施、配置或启用该服务。

**2026-09-17 T-056 文档漂移审计更正**：上一行「尚未实施」只适用于 2026-09-13。
T-048 已把该运行槽实现到 `crates/sync-transport`（状态机、持久 schema v1、Linux adapter、
装配层），现行实施记录见 [`owned-run.md`](owned-run.md)。仍然成立的是「尚未配置或启用」：
`linux::verify_backend`（`crates/sync-transport/src/linux.rs:198-215`）在所有平台都返回缺口，
所以本仓库至今没有真正跑起来的受控 Headless 进程 owner，也没有隔离运行环境或真实 Sync 接入。
另注：下文 §5 的 `start_once` / `wait -> Reaped | ExitUnconfirmed` 是本核查当日的**设计建议**，
最终冻结的形状不同（`prepare_once` + `start` 两步、`wait` 返回把 `phase` / `quiescence` /
`exit` / `stop_reason` 分开报告的 `RunObservation`），以 `owned-run.md` §1、§3 为准。

## 1. 可固定的版本与来源

本次可以固定 **npm `obsidian-headless@0.0.14`**，不必用浮动 master 代替发布物。

| 项目 | 本次核验值 |
| --- | --- |
| [官方 registry 元数据](https://registry.npmjs.org/obsidian-headless/0.0.14) | version `0.0.14`；gitHead `783aa551569245d7db862de187a9e45ee9bd2cf0` |
| [官方发布归档](https://registry.npmjs.org/obsidian-headless/-/obsidian-headless-0.0.14.tgz) | 260014 bytes；SHA256 `ef75298ed3a356dcb2a4de98c6efe10b2ac64b00556007118b6ac70535e73156` |
| registry SHA1 | `493ec855ab4c9b26943dafe0f8e6e86ac5ac0024`；已与归档实际计算值比较 |
| registry SRI | `sha512-S1d/hxLKvCUG2g5tRyXFkzPqMs3Ntw1tDyzoF2yfHGRuB4B+Mi3X2vgT8LbfQKrkEEi3LfJRdXtYzAVHcbpccw==`；已比较 |
| 发布 `cli.js` | 218234 bytes；SHA256 `c6307dc72c00bcf6f22093fb3e0eb91fdc417fc9dd05884ff2c36e5a19cd0196` |
| 发布 `package.json` | SHA256 `266d1ea75f27ea0fd5c008162978a5f95dd94888d9f4fc6a90e9d85702ce529e` |
| 发布 `README.md` | SHA256 `0fd0531a00f94c1afc6988cb76fd89cff8ba78480989fa801da623998be7df36` |

实际归档的三个文本成员与官方 master `0d0ec4364bfde6c715c539cf3555ff8272bb7a58` 完全相同；该 commit 是发布 gitHead 的下一提交，只更新 CHANGELOG。发布包声明 Node `>=22.0.0`、`commander=14.0.3`、`better-sqlite3=12.11.1`；包含 macOS/Windows birthtime native 文件，Linux 不用该 birthtime addon。[包声明](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/package.json)、[提交关系](https://github.com/obsidianmd/obsidian-headless/commit/0d0ec4364bfde6c715c539cf3555ff8272bb7a58)。

这不是整个安装闭包的供应链核验：Node、依赖及其 native 产物尚未固定/运行。最终部署须冻结完整本地安装目录与平台产物，不能在运行阶段用 npx 或自动安装解析新依赖。

## 2. 实际命令与 JSON 边界

以下从精确发布的命令注册核对；`<mirror>`、`<remote-id>` 等仅表示已经绑定的可信参数，不是 shell 模板。[官方 README](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/README.md)、[命令实现 L150–155](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L150)。

| 用途 | 精确 argv | 输出与副作用 |
| --- | --- | --- |
| 单次同步 | `ob sync --path <mirror>` | 默认单次；无 `--once`；该命令无 `--json`；有真实网络及镜像写入 |
| 持续同步 | `ob sync --path <mirror> --continuous` | 监视文件并循环；必须由自有进程监管 |
| 查看配置 | `ob sync-config --path <mirror> --json` | 不给修改选项时只返回本地配置投影 |
| 查看“状态” | `ob sync-status --path <mirror> --json` | 也是本地配置投影；不查询网络/队列/远端文件，也不验证 token 当前有效 |
| 修改配置 | `ob sync-config --path <mirror> --mode bidirectional --conflict-strategy conflict --file-types image,audio,video,pdf,unsupported --configs "" --json` | 此例是建议白名单，不是本次执行。修改配置；过滤器变化还会更新本地 state.db 的 pending 数据，必须在静止窗口进行 |
| 初次配对 | `ob sync-setup --vault <remote-id> --path <mirror> ... --json` | 验证账号/加密并写入配置；不是初次下载。JSON 模式不询问密码；E2EE 需另行受控提供密码 |
| 本地/远端列表 | `ob sync-list-local --json` / `ob sync-list-remote --json` | 前者本地配置；后者联网列 Vault，均无文件清单 |

`sync-status` 与 `sync-config` 的 JSON 精确字段为：

```text
vaultId, vaultName, vaultPath, syncMode, conflictStrategy,
deviceName, configDir, fileTypes[], configs[], excludedFolders[]
```

没有 `status/complete/pending/version/lastSync/perFileMax/receipt`。结果不含 encryptionKey、salt 或账号 token；这些安全删减不使它成为身份或同步证据。不存在公开的 `sync-status --file`、单文件 receipt 或全体远端文件 manifest 命令。JSON 是一个多行对象，不是 NDJSON。[投影 `_r` L155](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L155)。

精确版本内可识别的退出码：配置缺失常用 3，认证/加密前置缺失常用 2，其余同步捕获错误常用 1；Commander 的非法参数也可能是 1。不要只凭数字把任意错误归为认证失效。SIGINT/SIGTERM 触发 `stop()` 后也可能正常结束，因此取消状态必须由父进程自己记录，优先于最终 exit 0。

## 3. 身份配置、路径及进程边界

官方实现从以下根读取配置：[路径与认证函数 L140](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L140)。

| 平台 | 配置根 |
| --- | --- |
| Linux | `${XDG_CONFIG_HOME}/obsidian-headless`；未指定时 `${os.homedir()}/.config/obsidian-headless` |
| macOS / 其它 | `${os.homedir()}/.obsidian-headless`；此分支不使用 XDG_CONFIG_HOME |

根内 `auth_token` 保存账号 token。若进程环境存在非空 `OBSIDIAN_AUTH_TOKEN`，它优先于文件。每个 remote Vault ID 对应 `sync/<vaultId>/config.json`、`state.db`、`sync.log`；配置含镜像路径和 E2EE 密钥材料。相同配置根下同一 remote ID 只有一个配置槽，两个不同镜像不能当作独立客户端共用该槽。`--config-dir` 只是镜像内部 Obsidian 设置目录名，不能隔离账号/密钥/SQLite。

建议生产使用专用 OS 身份、固定私有 home/config 根、每个 link 独立实例范围；Linux 显式设置专用 XDG 根，macOS 使用专用 home 的子进程环境。不得修改宿主全局 HOME。运行器 `env_clear` 后只加入经过批准的运行变量，不继承 OBSIDIAN_AUTH_TOKEN、NODE_OPTIONS、NODE_PATH 或用户临时注入；runtime 不接受密码 argv。首次配对是单独人工配置动作，密钥配置文件只由 Headless 使用，SyncTransport 不读取/打印其正文。只通过安全投影核对 remote ID、mirror path、mode 与过滤项。

官方 token/config 的新建 mode 为 0600、部分目录为 0700，但已有文件权限未见统一修复；SQLite/log 创建也不统一指定该 mode。宿主仍需私有根、受控 umask、真实 UID/根 inode 核验与可写范围隔离。`sync.log` 是追加文件，源码未见轮转预算；运行器的 stdout 上限不能替代该磁盘容量预算。

发布 JS 没有导入/调用 child_process、spawn、fork、execFile 或 worker_threads；直接执行固定 Node + cli.js，预计业务处理留在该进程。同步结束会停止 WebSocket、SQLite、watcher/定时器；SIGINT/SIGTERM 使用同一 stop 路径。但 native 模块、依赖闭包与具体 OS 尚未动态验收，不能把“JS 未见派生”提升为“整棵进程树一定已退出”。[生命周期与信号 L146/L154](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L146)。

Headless 自有锁是 `<mirror>/<configDir>/.sync.lock` **目录**，靠 mtime 刷新和约 5 秒旧锁判定；不是 Studiolo 的 flock。它先释放该锁，再结束日志/stop 清理，故锁消失也不是进程终结证据。不要抢删它、不接管它、不以其 mtime 给自己的镜像安装发许可证。[锁类 `os` L146](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L146)。

官方 adapter 可解析文件/目录 symlink。专用镜像必须在运行前完整拒绝链接和特殊文件，并限制运行身份可读写范围；只把 Bridge 的 NOFOLLOW 检查放在运行后不能阻止 Headless 先跟随链接。[adapter L142](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L142)。暂停本地 child 不暂停远端设备，官方帮助也明确不应在同一设备混用桌面 Sync 和 Headless Sync。[Headless 官方帮助](https://obsidian.md/help/sync/headless)。

## 4. 完成与大小究竟能证明什么

### 首次下载与单文件

`sync-setup` 只完成配置。内部收到 server ready 时就清除 `initial`，而文件队列另行处理；把 state.db 的 `initial=false` 单独读出仍不足以证明正文已齐。`Fully synced` 表示当次算法的过滤/跳过规则下队列趋于空；过滤文件、非法远端路径、超限文件、部分扫描/合并错误均可能被排除。人类日志还包含未转义文件路径，不能靠匹配一行字符串创建可信完成票据。

`Upload complete <path>` 出现在内部 push 返回后，但公开输出不绑定不可变 request ID、实际摘要、远端版本或持久回执，也不能证明后来没有远端覆盖。底层有逐块响应/服务器变更序号，但没有公开 CLI 结构化导出。它最多是诊断线索。没有依据把 exit 0、本地相同 hash 或这行日志转换成 Bridge 某个未知原键的成功 ACK。[同步循环/上传/下载 L146](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L146)。

可分别记录三种证据：受控 child 已被回收；完整本地镜像扫描在某个静止窗口取得；独立接收设备对已知文件实际读回字节。它们不等同于全体远端文件清单原子快照。首次没有已验证覆盖清单时只能保持引导未完成/create-only，不能授权删除。是否引入精确版本的只读 state.db 证据适配器或向官方争取结构化 manifest/receipt，需后续独立冻结，不能在此偷加内部数据库权限。

### 200 MB 的确切单位

官方帮助明确 Plus 每文件上限标称 **200 MB**，Standard **5 MB**，未在该公开说明定义十进制或二进制字节换算。[官方套餐与存储限制](https://github.com/obsidianmd/obsidian-help/blob/master/en/Obsidian%20Sync/Plans%20and%20storage%20limits.md)。

精确 0.0.14 客户端初始化 `perFileMax` 为 **199 × 1024 × 1024 = 208666624 bytes**；连接后可被服务 `init` 响应中的非负整数字段覆盖。上传前将文件 `stat.size` 与该值比较；显示格式用 1024 换算但标签仍写 MB。由此只能证明客户端值以 bytes 比较及默认值，不能证明生产 Plus 服务当日发的是 200000000、209715200 或 208666624；默认 199 也不能自行解释为服务承诺或加密预留。[上限与比较 L146](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L146)、[显示函数 L141](https://github.com/obsidianmd/obsidian-headless/blob/783aa551569245d7db862de187a9e45ee9bd2cf0/cli.js#L141)。

因此 Transport 的 `max_file_bytes` 仍应显式配置并标明来源/核验时间，未知时不冒充实际最大支持。真实边界验收需测试候选阈值前后并在独立接收端读回；不能只看发送端退出码。客户端上传一次 readFile 整个文件、加密后再按 2 MiB 分块；下载也先按服务长度分配 ArrayBuffer。**分块网络传输不代表有界流式内存**，Broker 的流式上限不覆盖这个外部进程，须给它单独内存/磁盘预算并实测大文件峰值。

## 5. 建议最小受控子进程 API（本节为主线设计建议）

只允许三类运行操作：`InspectConfig`、`ApplyApprovedConfig`、`RunOnce`。第一片可不暴露 Continuous；需要时仍沿同一 OwnedRun 生命周期。配对/登录/删除远端/Publish/任意 flags 不在 runtime API 内。

```text
PinnedRuntime { node identity, cli/package-tree digest, version=0.0.14 }
TransportBinding { link, remoteVaultId, mirrorRootToken,
  privateConfigRootToken, approvedConfigDigest, scopeHash, scopeEpoch }
RunBudget { deadline, stopGrace, stdoutBytes, stderrBytes,
  configJsonBytes, privateStateBytes, logBytes, memoryBytes }
start_once(binding, expectedRunSequence, budget) -> OwnedRun
request_stop(OwnedRun) -> cancellation recorded
wait(OwnedRun) -> Reaped | ExitUnconfirmed
```

`Reaped` 保留 run ID/序号、绑定摘要、固定程序摘要、开始/结束时间、原始 exit code 或 signal、父进程取消/超时原因、输出截断标志与安全分类；不设置 `Synced` 或 per-file committed。命令曾启动而结果不明须记录 Unknown，不自动以新 run 覆盖原 action pending。Headless 没有传入 Bridge ActionKey 的接口；重复传输只能是重新观察/协调，不能假装对原请求幂等核对。

过程契约：

1. 获取自身实例租约并核验真实 UID、固定程序和所有根；无其它本实例运行。参数数组直接 exec 固定 Node/cli，禁 shell、PATH 搜索和 npx；stdin 关闭。配置查询也只在该串行域执行。
2. 精确校验 config JSON 的全部字段/类型/枚举、路径与 remote ID；键缺失、额外键、过大/多个对象或截断均拒绝。不从日志读授权；不把默认 fileTypes 当作完整附件授权，必须核对 approvedConfig。
3. 子进程启动前登记 run 意图并持久化 binding/sequence；限制 stdout/stderr 内存且持续排空，超限按 Budget 取消/未知处理，不能截断后假成功；私有配置根自身另有实际磁盘预算。
4. `wait` 必须等待真实自有 child 终结，并由平台层确认本运行容器/子进程组已静止。Linux 可采用专用受控 cgroup/服务 scope；普通 PID 名称搜索不是可靠身份。进程组方案还需排除 escape/复用风险。若平台不能给退出边界，返回 ExitUnconfirmed，禁止镜像安装；不能终止别人的进程来换取该边界。
5. 取消/超时先只停止自己持有身份的 child/运行容器，给有界 grace；需要强停也只作用于该范围。即使最后 exit 0，父取消仍是 Cancelled。pipes EOF 或 `.sync.lock` 消失不替代真实 wait/静止证明。
6. 只有静止后才发内存 `QuiescentMirror` 能力，其绑定当前实例 epoch、run、root pins 与排他租约；用于白名单 CAS 安装。安装回执只证明本地 mirror 的持久效果，必须显式 `remote_delivery=unknown`。以后再 start child，立即使该静止能力失效。
7. 镜像外 stage、逐路径 expected revision、原 ActionKey 和审计保持既有 Bridge/Broker 规则。重启时原 child 是否仍活着不能从旧 PID 猜测；运行容器核对失败保持未知并阻断安装。
8. 成功跑完后重新完整扫描镜像，文件摘要和覆盖证明由 own adapter 生成；不以此次进程结果直接推进共同 baseline。远端/设备送达需独立证据，永远不以手机在线与否猜测。

预算数值应由主线冻结并在真实 200 MB 级测试后确认。上述 API 必须显式接受预算，不在本核查里把任意常数默认为生产承诺。

## 6. 最小验证顺序与外部输入

以下是**待执行计划**，本次没有运行任何夹具或真实账号。

| 层 | 最小场景 | 可以验收的结论 |
| --- | --- | --- |
| 运行器真实子进程夹具，不用 Headless | 正常/非零退出；SIGTERM 后 exit0；忽略信号；孙进程保留管道/继续写镜像；父崩溃；PID 重用；超量 stdout/stderr | 自有 child/后代退出边界、超时、预算与禁止过早安装；不能证明 Sync 协议 |
| 精确发布本地隔离，不用账号 | 在专用空 home/config 下运行 version/help/status；构造纯合成 config 用 status JSON；全套 argv 的可接受/拒绝；查询不得触碰宿主配置 | 真包参数和 JSON；status 可成功不意味着在线/认证有效 |
| 精确发布 + 专用受控远端测试 Vault | 从预知 manifest 下载中文路径/正文/各种附件；独立接收端逐文件摘要；观察真实进程树/FD，连续及单次停止、强停重开、网络断开 | 该版本/平台/配置下的实际完成范围和进程生命周期；不能推为任意 Vault 原子快照 |
| 并发与中断 | 安装前后远端同时修改、A→B→A、同键重开、删除对修改、下载途中终止、配置范围变化 | 未知保持、基线不提前推进、create-only/冲突/删除保护；不能补造旧 ACK |
| 大附件 | 200000000、208666624、209715200 bytes 周边的最少区分样本；最终确认阈值−1/等号/+1；测试在专用 quota 内安排并读回 | 当时账号/服务实际可传字节边界；发送日志与 exit0 不能代替读回 |
| 资源与隔离 | Headless 峰值 RSS、内部 state/log/镜像容量、磁盘满、symlink/FIFO/根替换、native module 失败 | 实际预算及能力隔离，且故障不能触碰 canonical/宿主 home |

真实层所需输入：获准测试账号/Plus 订阅、专用远端 ID、人工配对后的私有身份目录、固定部署 OS/Node/依赖闭包、空专用镜像/外 stage、完整初始 manifest、独立接收端和授权网络、磁盘/内存/执行时限预算。E2EE 密码不交聊天或日志；测试不触碰生产 Vault。实际手机往返和两周试跑仍是单独外部验收。

## 7. 给主线的结论

现在可实施固定版本的受控进程 owner、配置投影核验、静止窗口与本地 CAS 安装证据。**单靠 0.0.14 公开 CLI 不能实现可信全量远端覆盖证明或原 ActionKey 绑定的单文件远端回执。** 自动删除前的首次覆盖证明、实际 Plus 字节阈值、完整进程闭包及资源预算，是明确的剩余门槛；不要把 process exit 0 或 status JSON 设为它们的替代品。

本核查不改变权威既定的“双向 Bridge、唯一 canonical Broker、自有 Headless 退出后镜像安装、暂停本地不冻结远端”。
