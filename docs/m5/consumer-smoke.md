# 当前 Codex 会话的 M5 实际使用记录

2026-09-20 04:26:28–04:30:37 UTC。用户要求「用你自己来真实测试一下」。

当前会话实际通过 HTTP MCP 使用本分支编译的 `studiolo-agent`，后端为私有 Unix 黑板入口、
持久 Control DB、CanonicalWorkbench、FsVaultBroker 与 SQLite 审计。请求内容和操作顺序由
当前 Codex 逐步决定；Python 只负责协议、私有凭据和记录，没有预置通过断言或模拟服务响应。

结论：**这次 M5 接入方闭环通过**。共记录 19 个 MCP 动作（2 次 initialize、17 次工具调用），
14 个正常返回、5 个预期拒绝。不是生产部署、完整 M5 全项验收或 M6 RG-RUNNER 验收。

## 实测环境与身份

| 项目 | 实际值 |
|---|---|
| 环境 | 当前 Mac，本分支未提交工作树；独立临时验收库 |
| 工作树 | `.claude/worktrees/codex-m5-m6-blackboard-runner`，基线 `b42893f` |
| 数据根 | `/private/tmp/studiolo-codex-01a0bc6f-c` |
| 实际 MCP URL | `http://127.0.0.1:65202/mcp`，测试结束已停止 |
| 当前会话实例 | `codex / 01M2YH1HZMHE3VCD9SHTEBJ16T` |
| 工作项 | `01M2YH2E6QAV58N0Q4XK22VBDP` |
| attempt / fence | `A-01M2YH3BQ84032CNBEXTDR1FPJ / 1` |
| 终态 | `succeeded`，无 active attempt；最后路牌显示本实例持有 0 项 |
| 初始策略 hash | `7c50c1e2e79f51d0128dc9ed167ed03aa2d574ff08a48db52548452bd724ae28` |

此会话原本没有配置 Studiolo MCP；没有改写用户 Codex 配置。通过一个小型命令行 MCP 客户端，
当前会话调用真实服务。临时库为全新目录，首版 bootstrap 配置 Claude/Codex 仅可写 Studiolo，
Grok 只读。Grok 身份是本会话发起的负向探针，不能写成“真实 Grok 模型已验收”。
没有使用真实 Vault、个人模型登录目录、专用 API key，也没有替人侧伪造 L3 会话。

## 实际结果

| 检查 | 结果 |
|---|---|
| MCP initialize、冷启动路牌 | 识别为 Codex，服务端登记实例，路牌包含实时黑板与策略版本 |
| 初始协调板读取 | 读到预留人工行 |
| 创建与认领 | 服务端绑定实际实例、策略 hash、attempt/fence、10 分钟 lease 与路径范围 |
| 只读身份写入 | 拒绝：`能力集未授予黑板写权限` |
| Codex 向 Other 项目写入 | 同样拒绝；Other 仍在读取白名单，证明读/写范围独立 |
| 批次预览 | 返回 before/after diff、命名空间化 batch ID 与 fingerprint |
| 篡改预览 fingerprint | 拒绝：`idempotency key was reused with different input` |
| 使用原预览提交 | Broker 返回真实 mutation revision 与 batch receipt |
| 相同批次重复提交 | 返回完全相同的 receipt；没有重复写入 |
| MCP 读回证据 | 正文逐字匹配提交内容，SHA-256 匹配 |
| 人工协调行 | 运行中与终态两次读回均保留；生成区按任务状态更新 |
| 只读身份读取工作项 | 成功，证明只读身份未被整体禁用 |
| 缺少 closeout 的回执 | 拒绝：`请求字段缺失、多余或类型无效` |
| 完整逐项回执 | 接受，状态变为 `succeeded`；非 M6 人工候选审批 |
| 终态后沿用原 claim 写入 | 拒绝：`claim已过期或不属于当前实例` |
| 最后路牌与投影 | 实时持有 0 项，TASKS 生成区为 `succeeded` |

证据产物：`项目/Studiolo/05-验收/current-codex.md`。
Broker revision：`a0feb97d7b636bbc543704e8af657687b2b41af3e335821f8b44f33ac124b60c`。
正文 SHA-256：`6f03f93034d2ef111147529cd5425e4b9240c1eb12564ae7eed88ca76f809abb`。

回执的 canonical revision 来自 batch receipt；`read_note` 的供给 revision 不能冒充 canonical
写入前置条件。正文 SHA-256 从实际 MCP 读回的 UTF-8 字节计算，再逐字比较提交内容。

## 证据保留与清理

本机数据根保留 `transcript.jsonl`、`vault/`、`board/`、`audit/`、`agent/audit/` 和请求 JSON。
transcript SHA-256：`9157fdb943505722ebc5e804b4e0a42edd3f38b55911ad37d978b0a67dc08e65`。
记录不含 Authorization header 或 MCP session header；结束前逐项确认临时 bearer/session 值未混入。
退出时库中 canonical audit 97 条、agent audit 26 条（包含准备与权限/读取事件，不等于工具调用数）。

两个服务均 Ctrl-C 正常退出（exit 0），Unix socket 已移除，MCP 监听已关闭。
所有本次生成的 bearer 文件和客户端 session 文件已删除；数据与审计保留供复查。

## 新增可复用入口与验证

- `crates/local-server/examples/consumer-smoke.rs`：只接受全新绝对目录，创建临时身份/策略/内容，
  启动真实黑板和维护循环；输出 `agent_argv`，由正式 agent 二进制提供 MCP；30 分钟上限。
- `scripts/consumer-smoke.py ROOT SESSION IDENTITY METHOD < params.json`：一次请求一个动作，
  只连接描述文件中的数字 loopback 地址，令牌留在私有文件，支持实际 SSE 空事件/多行数据。
  JSON-RPC `error` 是协议拒绝，须检查响应，不能仅以进程 exit 0 判断操作成功。

已执行：两项实际二进制构建、本记录的 19 个动作、`cargo fmt --all -- --check`、
Linux `cargo --locked clippy -p studiolo-local-server --example consumer-smoke -- -D warnings`，均通过。
Python 语法解析通过；没有修改生产实现或重新运行无关全库回归。

准备阶段修正了两个测试入口问题：种子文件复用 idempotency key，以及客户端未处理 SSE 空事件。
Mac 沙箱拒绝绑定 Unix socket 后，按用户授权用工具批准的本机监听权限启动。未放宽产品权限检查。

## 未覆盖

当前 Codex 是外部 Consumer。它的推理与工具执行未经过 Studiolo Gateway，也未被 Studiolo
放入专用 Linux cgroup/bwrap，所以这次不能证明 M6 专用凭据、费用硬限制、Runner 无直连外网、
Claude/Codex CLI 上游兼容性或完整取消隔离。正式环境的策略 L3 发布、真实 Sync/备份和客户端
安装也未执行，仍按 [真实环境验收清单](../m6/release-acceptance.md)继续。
