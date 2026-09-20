# 本机 Codex / Claude Code 派发

当前实现位于 `codex/m5-m6-blackboard-runner` 开发工作树，未提交、合并或部署。用户最新约束是调用自己的本机原生客户端，只用订阅额度；此前服务器 API Gateway 的认证路线已停用。

## 已实现的链路

```text
Studiolo 黑板 → 本机派发面板 → 已配对的 loopback 连接程序
  → Codex queue / Claude Channel → 已登记的原生会话
  → MCP 获取权威任务与规则 → claim → heartbeat → return
  → canonical 黑板与 Broker → 面板核对真实终态
```

本机客户端按 ADR-013 接入方运行。它使用自己的登录和本机工作目录，保留原生审批；不冒称服务器沙箱执行体。Studiolo 不读取、复制或转发模型账号 token，不接入模型 API。

- `scripts/studiolo-local.py`：启动、登记、诊断、配对码及每会话 MCP 配置入口。
- `scripts/local_client/`：私有配置、MCP HTTP 客户端、SQLite 通知账本、原生适配器、stdio/Claude Channel、精确黑板工具 schema。
- `web/src/LocalDispatch.tsx`：黑板内的「本机 Codex / Claude Code」面板，创建只读工作项或向指定客户端派发现有工作项。涉及写入的工作项由已有 M5 接口定义独占路径后派发。
- `crates/local-server/src/security.rs`：CSP 只额外允许字面 `http://127.0.0.1:*`；本机服务仍要求固定 Origin 和配对码。未开放 LAN 或任意网站。

任务书只保存在权威黑板。通知只含项目、工作项 ID、通知 ID 和固定取任务说明，不传任意 shell 命令。仅登记的 Claude/Codex 可写 `项目/Studiolo`，未指定的类型维持只读。

## 真实验收（2026-09-20）

使用独立新建的真实 Broker、BoardPlane 和 `studiolo-agent` MCP 服务，未连接生产 Vault。以下不是模拟模型或手工代写回执：两家原生模型都从通知开始自行完成 MCP 操作，主线随后独立读取 canonical 结果。

| 原生客户端 | 送达方式 | 工作项 | 最终核对 |
|---|---|---|---|
| Codex 0.155.1，ChatGPT 登录 | `codex queue` → 本机 app-server 中已存在的专用验收会话 | `01M2YQTZATXPVSWGMZGKNHDM75` | claim、heartbeat、return 成功；task `succeeded`，receipt `completed`，通知账本 `completed` |
| Claude Code 2.1.278，Claude Max | 原生交互会话中的 `notifications/claude/channel` | `01M2YR0EGSH1Q660GE4V371T0G` | claim、heartbeat、return 成功；task `succeeded`，receipt `completed`，通知账本 `completed` |

Codex 受测会话 `01a0bd7d-6f12-7e51-9e06-6a1352ad6c26`；Claude 受测会话 `e39da1e1-2422-496b-81a8-f813387a952e`。Claude attempt `A-01M2YR0QVP5KZR64JWEBB8AGFH`、fence 3；两项都是只读验收，文件写入数为 0。文件产物/Broker 批次路径另有 M5 实测证据，见 [消费端验收](../m5/consumer-smoke.md)。回执中的耗时/工具计数属于客户端自报，不能当作计费或精密计时证据。

计费核对：Codex 官方 `account/read` 为 `chatgpt`；`account/rateLimits/read` 为 `hasCredits=false, unlimited=false, balance=0`。Claude 官方认证为 `claude.ai / firstParty / max`，原生 `/usage` 显示 `Usage credits are off`，当次会话配额有余。未提供模型 API key、未充值、未修改付费设置。

初次测试发现两个真实问题并修复：验收宿主需要正常处理原生 MCP 工具审批；上游 `board.request` 泛对象不足以指导模型，现已提供精确嵌套 identity 与完整回执 schema。失败试跑没有被计作通过，最终两项使用修复后的链路重新验收。Claude 第一次启动的测试包装器把启动代码从 stdin 误送成提示词，产生一条短回复后退出；最终验收改用脚本文件启动、独立空会话及真正的 Channel 通知，未以该误触发作为通过证据。

权威回执快照已保存到仓库：[Codex 回执](evidence/local-codex.json)、[Claude 回执](evidence/local-claude.json)。

无凭据临时证据目录：`/private/tmp/studiolo-local-dispatch-01a0bc6f-b/`，其中 `local4/receipt-evidence.json`、`local4/native-events.jsonl` 和 `claude-local/receipt-evidence.json`、`claude-local/delivery-evidence.json` 保留权威回执与原生工具证据。临时服务和测试令牌在验收收尾关闭/移除；这些目录不是长期服务配置。

验证边界：已验证原生本机会话的派发闭环；未声称当前 Codex 桌面任务已安装该 MCP、任意已打开的 Claude 会话可以被热插入 Channel，或线上 Studiolo 已发布新界面。Claude 需要启动时明确加载开发 Channel，并保留官方确认界面，见 [Claude Channels 官方说明](https://code.claude.com/docs/en/channels-reference)。

## 本机运行

需要 Python 3.10+ 和已安装、已订阅登录的官方 CLI。运行程序在用户本机；无需到另一网络、无需登录 githubci、无需提供模型 API key。

`--studiolo-token-file` 指已有 Studiolo 接入身份令牌，是访问黑板的凭据，不是模型 API key。文件及配置要求当前用户所有、0600；状态目录为 0700。`--origin` 为实际 Studiolo 页面 origin（仅 scheme/host/port，不带路径）。一次登记一个客户端，可用 `add-client` 追加，所有路径由本机登记固定，网页请求不能更改。

```bash
# 先设置实际配置路径，再按 --help 提供已登记的黑板地址/身份、工作目录和会话 ID。
python3 scripts/studiolo-local.py --config /绝对路径/local.json init --help
python3 scripts/studiolo-local.py --config /绝对路径/local.json add-client --help

# 核对原生登录；付费声明缺失或登录方式不符时拒绝派发。
python3 scripts/studiolo-local.py --config /绝对路径/local.json check

# 终端一：保留连接程序运行，默认只监听 127.0.0.1:4318。
python3 scripts/studiolo-local.py --config /绝对路径/local.json serve

# 终端二：打开指定本机会话，MCP 设置只用于本次启动，不改个人全局配置。
python3 scripts/studiolo-local.py --config /绝对路径/local.json launch --client 本机登记ID

# 在自己的终端查看配对码，填入 Studiolo 黑板的本机面板。
python3 scripts/studiolo-local.py --config /绝对路径/local.json pair-code
```

Codex 登记 `provider=codex`、`mode=codex-queue` 和具体 `thread` UUID，可选本机 `unix:///…` app-server 地址。Claude 登记 `provider=claude`、`mode=claude-channel`，可选 `thread` 以恢复指定会话。`launch` 保留正常审批；不会自动批准所有命令。已在运行的会话需先按原生客户端流程加载本 MCP，不能由外部程序悄悄插入工具。`client-config --client …` 可输出无令牌的 Codex TOML 或 Claude JSON 配置。

首次登记必须明确 `--paid-usage-disabled` 才能派发。它记录用户核实后的本机声明，不是假装存在的平台强制开关：Codex 核对无额外 credits，Claude 核对 usage credits 关闭。原生登录检查会拒绝 API/第三方 provider，并从连接程序启动的子进程清除模型 API/令牌/路由环境变量；它不能修改已运行会话的环境或账号付款设置。不能确认额度边界时停止，不自动转 API、充值或消耗 reset credit。

## 状态、恢复与权限

`pending → dispatching → sent` 只表示通知投递。只有重读权威黑板并观察到完整终态回执，才记 `completed`。运行状态另显，失败回执不会伪装为任务成功。

- 同一 request ID 同参幂等，不同参数冲突；同任务重复点击不重新投递。
- 发送超时、非零退出或发送途中崩溃记 `uncertain`，不自动重放。创建响应丢失也不自动重新创建。
- 本机进程有单实例锁；重启恢复未确认投递，不能抢另一活动进程的通知。
- 原生 MCP 断线后就绪状态过期。黑板不可达、类型撤权、项目不符、已被他人认领或政策变化时拒绝新派发；实际写入仍由 M5 的身份、claim/fence/lease/策略检查裁决。
- 每客户端独立本机桥接令牌，不能调用 UI 管理入口；UI 配对码不能冒充另一个客户端进行 MCP 调用。配对码仅在网页内存中，刷新/断开后消失。
- `cancel` 通知只取消未投递项。已发送任务需通过权威黑板撤销，原生客户端应在后续读/心跳/写回时停下；不把停止共享用户会话当作默认取消办法。
- 不接受远程任意可执行文件、工作目录、命令、会话或项目。仅本机私有配置能登记这些值。上游 MCP 拒绝重定向与环境代理，HTTPS 校验正常开启。

## 回归命令

```bash
python3 scripts/test-local-client-core.py
python3 scripts/test-local-client-native.py
python3 scripts/test-local-client-server.py
python3 scripts/test-local-client-contract.py
cargo test --locked -p studiolo-local-server --test http_contract
cd web
npx vitest run src/LocalDispatch.test.tsx src/BoardBoundary.test.tsx
npx tsc -b
npx vite build
```

Python 的完整 JSON Schema 验证用例在已有 `jsonschema` 库时执行，缺库时明确 skip；不为运行连接器引入第三方依赖。正常 CI 只跑本地契约测试，不会调用订阅模型。真实模型验收使用本人原生登录并单独核对付费边界。

本轮回归结果：githubci 原生 Linux 上 60 项 Python 检查全部通过（包含完整 JSON Schema 验证）；Mac 54 项通过、6 项因未安装可选 jsonschema 明确跳过。相关前端 30 项、Rust HTTP 契约 5 项通过，TypeScript、前端构建、定向 ESLint/Prettier 和差异空白检查通过。未以本轮定向结果声称整个 M5/M6 或生产发布的全量验收。
