# M3 资料同步与新鲜度（T-036）

2026-09-09：本片实现 Mac 白名单资料到服务器的自动副本同步及 MCP 新鲜度。它不是 ADR-015 向各客户端生成 CLAUDE.md / skills / 工具配置的推模式，后者仍待实现。

## 实现计划

1. 沿用 `sync-agent-vault.sh` 入口，源端捕获可验证的完整批次；源文件改变则本次失败重试。
2. SSH 接收端在私有暂存目录验证完整清单，发布不可变代次，原子替换固定状态文件；并发发布互斥，失败不覆盖上一成功批次。
3. `studiolo-agent serve --replica-root <目录>` 与直接 `--vault` 模式互斥；供给层通过既有只读 Broker 验证清单，四工具共用已验证的内存代次。
4. 握手预算计入新鲜度；搜索、目录、正文均给出同批版本。过期/最近同步失败可明确读取最后验证副本，控制文件无效或内容漂移则拒绝内容。
5. macOS launchd 每 60 秒运行一次；生成可检查的安装文件和操作说明，实际启用与服务器换版单独验收。

## 协议 v1（源端、接收端、Rust 消费端共用）

- 默认白名单仅 `项目/Studiolo`、`_规则`；隐藏项完全不捕获；拒绝链接、特殊文件、非 NFC 路径、重复/大小写碰撞、越界路径。
- 默认服务器根 `/home/maxsong/.local/share/studiolo-agent-replica`，与既有副本分开；目录 0700、文件 0600，同 owner 且拒绝符号/硬链接。
- `control/state.json` 是唯一发布指针；`releases/<generation>/manifest.json` 与 `releases/<generation>/vault/<allow>/…` 是完整不可变内容；generation 为 32 位小写十六进制 UUID。
- manifest 为 UTF-8 JSON：`{"schema_version":1,"allow":[...],"entries":[...]}`。allow 与 entries 按路径排序，entries 包括白名单根以下所有可见文件/目录，不含白名单根本身；中间目录（如 `项目`）仅用于容纳白名单。条目为 `{"path":"...","kind":"file|dir","bytes":整数,"sha256":"64位小写hex"或null}`，目录 bytes=0、sha256=null。文件摘要覆盖原始字节，`source_digest` 覆盖 manifest 文件原始字节。
- state 为 `{"schema_version":1,"status":"ready|failed","last_attempt_at_ms":整数,"last_error":null或"transfer_failed|validation_failed","receipt":回执或null}`；回执为 `{"generation":"...","source_digest":"...","source_observed_at_ms":整数,"received_at_ms":整数}`。时间均 Unix 毫秒；不得把服务器重复扫描时间作为资料确认时间。
- ready 必须有回执且 last_error=null；failed 必须有固定错误原因。last_attempt 不早于成功回执时间，成功确认不允许回退。扫描失败后如指针已更换，按被新代次替代处理并有界重试，不把正常清理误报为内容漂移。
- 传输为 tar 流：根 `manifest.json`、`request.json`（`{"schema_version":1,"source_observed_at_ms":整数}`）、`vault/<路径>` 文件及目录；接收端逐项校验类型、路径、唯一性、长度及摘要，全部成功才发布。不直接调用 tar 解包到生产目录。
- 上限：manifest 4 MiB、25,000 条目、单文件 16 MiB（UTF-8 Markdown 限 4 MiB）、Markdown 合计 16 MiB、总文件内容 128 MiB。Markdown 单独限制，以容纳 512 MiB 服务中刷新时共存的新旧快照和读模型；附件只在验证时逐文件读取。所有扫描/传输有界。同步超时 120 秒，源端/接收端都不允许并发发布；默认每 60 秒重试。
- tar 原始字节流及头部同样有界，不能通过 PAX/GNU 扩展头绕过限额。接收器只识别协议明确允许的成员，不将归档路径直接交给解包器。
- 新鲜度阈值默认 300 秒，可用 `--replica-max-age-secs` 显式配置 60..=86400。source_observed 与 received 两者取较老的时间计算 age；超过当前时间 30 秒、逆序或不合法时间拒绝。失败状态保留上次成功回执。断网/崩溃由 age 过期暴露，不伪造远端已收到失败通知。
- 源端每轮重新校验，未变内容仍产生可信确认；接收端可在验证上一代完整内容后复用相同 digest 的代次，仅刷新回执。旧代次有界保留（当前及上一代），消费端已经持有的内存内容不受清理影响。

## 验收要求

- 修改/新增/删除/重命名及空目录同步一致，白名单外和隐藏内容不进入传输。
- 源扫描中修改、半传输、错误摘要/成员、链接替换、并发同步均不发布半批次。
- dry-run 不连接或改动服务器；检查结果不输出笔记正文或凭据。
- 真实 loopback MCP：新代次进入后四工具一致，旧代次无混读；过期/失败明确标记；漂移/无效状态拒绝内容；审计与 2000 token 硬门槛不被绕过。
- 本地和 Linux 测试、格式、clippy、架构守卫、脚本回归通过后才打包；线上启用与真实客户端验收单独记录。

## 2026-09-10 本地验收记录

基线 `49a9f1a`，工作分支 `codex/m3-replica-freshness`。以下均为本轮实际执行结果，不沿用 T-034 部署证据。

| 环境 / 范围 | 结果 |
|---|---|
| macOS Rust 工作区 | 去重覆盖 856 项运行时测试及 6 项文档测试；全工作区 fmt 与严格 Clippy 通过 |
| Linux aarch64，Rust 1.97.1 | vault-broker、supply、agent-server、studiolo 四 crate 共 194 项运行时及 2 项文档测试通过；全工作区所有目标严格 Clippy 通过；agent 构建通过 |
| Python 同步回归，两平台分别执行 | 源端 20、接收端 15、边界 9，共 44 项通过；自动任务生成器另 3 项通过 |
| 既有 MCP 检查器，macOS | 8 项通过 |
| Python → Rust 真实 loopback 联调，两平台分别执行 | 下列 6 阶段全部通过 |
| 真实 Mac Vault 仅本地 dry-run | 白名单捕获 98 文件、17 目录、1,306,245 字节；未连接服务器。该数量为档案收尾前快照 |
| 独立自动同步配置 | 8 个同步源文件与当前仓库字节一致，plist 语法检查通过；未加载 launchd |

macOS 首次全工作区运行中，既有 Auth 并发测试 `revoked_pending_pins_session_until_exact_delivery_and_late_ack_cannot_switch` 超时；该测试单独以 `--test-threads=1` 重跑通过，未改 Auth 实现。其余工作区以 `--exclude studiolo-auth -- --test-threads=4` 全部通过，Auth 文档测试另跑 3 项通过。因此上表是分次验证后去重覆盖的口径，不是一次无失败的全工作区运行。

联调使用临时虚构 Vault、临时私有身份库和独立回环端口，验证：

1. 实际 Python 捕获与接收发布后，由 Rust 握手及四工具读取同一批次，并拒绝匿名请求。
2. 更新、增删文件及空目录后，立即读取新代次，不依赖后台轮询。
3. 半传输被拒绝，旧完整回执保留且标为同步失败；重试恢复。
4. 重启服务后保留来源确认时间；超过阈值的握手和正文均标为陈旧。
5. 同大小字节篡改被发现，四工具均停止供给内容。
6. 源内容未变时重新同步，可发布新代次修复已损坏的服务器副本。

Linux 使用固定镜像 `rust@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97`。本轮未执行服务器 x86_64 原生验收，也未触发远端 CI、生成发布包、合并推送、安装自动任务或更换线上服务。启用与回滚按 [操作说明](replica-operations.md) 执行，T-036 的实际启用验收仍待完成；完整 M3、客户端推模式及 M2 canonical 同步均不在本次完成范围。

本机日志保存在 `/private/tmp/studiolo-t036-macos-tests.log`、`/private/tmp/studiolo-t036-macos-remaining.log` 与 `/private/tmp/studiolo-t036-linux.log`；这些临时文件可能被系统清理，CI 已登记相同回归入口，供后续复验。
