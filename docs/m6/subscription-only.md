# M6 仅订阅额度：当前约束与适配边界

> 用户随后明确目标是调用自己本机已登录的 Codex、Claude Code（包括当前这样的会话）。当前主线以 [local-client-dispatch.md](local-client-dispatch.md) 为准；下文服务器隔离 Runner 的认证约束保留为历史分析，不能作为本机接入方派发的前置条件。

2026-09-20，用户明确「只允许用订阅额度，不使用API」。此决定覆盖本任务此前的专用 API 凭据方案。允许消费的是已包含在订阅中的额度，不授权 API 计费、额外 credits/usage credits、自动充值或付费回退。

## 已执行的变更

- 仓库和 githubci 私有准备目录里的 `configure-githubci-keys.py` 均替换为固定拒绝入口，任何参数均退出 1，不读取凭据、不写文件、不启动服务。实际远端负向检查通过；旧的 `录入 && 启服务` 命令不会越过第一步。
- 实际回读 `studiolo-m6-keyholder.service` 为 `inactive/dead`，没有 socket，没有发起模型调用。API 接入步骤已从当前验收文档撤销。
- 不读取或迁移个人 CLI 登录目录；本轮未执行订阅登录、提取 OAuth、API 调用或修改用户的付款设置。
- 已通过的两家 CLI 隔离、资源上限、raw timer 与 26 项 Runner 回归证据保留。它们不证明订阅认证或真实模型调用已经成功。

## 官方支持的认证与当前实现的差别

| 项目 | 已核实 | 当前 M6 的缺口 |
| --- | --- | --- |
| Claude Code 原生登录 | 未改动的官方 CLI 允许用户用自己的订阅登录；SSH 场景可由用户在浏览器完成官方流程 | 当前 `--bare` 明确跳过 OAuth，不能只填一个不同类型的凭据 |
| Codex 原生登录 | ChatGPT 订阅、`codex login --device-auth`、非交互 `codex exec` 均有官方支持 | 当前强制自建 `relay` provider 与短票，隔离 HOME 没有订阅登录态 |
| 账号凭据边界 | 普通 CLI 登录把账号凭据交给 CLI；Codex app-server 外部认证也接收真实 access token | 不等同于当前「Gateway 持账号凭据，Runner 只有 attempt 短票」 |
| 逐请求控制 | 现有 Gateway 解析两家 API 协议并做 fence/lease/policy、预算和封存 | 原生订阅流量并未适配，不能把 TLS CONNECT 代理、直连网络或更换 URL 宣称等价 |
| 套餐额度 | 原生订阅登录可以消费套餐额度 | 仅排除 API key 不足以排除额外 credits，需要账号设置与实际响应验证 |

Claude 的来源：[官方认证](https://code.claude.com/docs/en/iam)、[无头与 bare 模式](https://code.claude.com/docs/en/headless)、[原生 CLI 登录与凭据使用边界](https://code.claude.com/docs/en/legal-and-compliance)。`--bare` 不读取订阅 OAuth；去掉该参数还会恢复自动发现 hooks、MCP、skills 等，因此不能只删参数而不重新验证隔离和配置加载。

Codex 的来源：[官方认证](https://developers.openai.com/codex/auth)、[非交互执行](https://developers.openai.com/codex/noninteractive)、[app-server 认证](https://developers.openai.com/codex/app-server)。固定 0.155.1 的 [external_auth.rs](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/app-server/src/external_auth.rs) 保存实际 `CodexAuth`；[network credential broker](https://github.com/openai/codex/blob/rust-v0.155.1/codex-rs/network-proxy/src/credential_broker/providers/openai.rs) 的 OpenAI 实现针对 API key，不是已验证的订阅 OAuth 适配器。

## 计费边界

Claude 官方说明，开启 usage credits 后可以在套餐用完后继续按量付费；可在 Settings → Usage 关闭。详见[官方 usage credits 说明](https://support.claude.com/en/articles/12429409-manage-usage-credits-for-paid-claude-plans)。本轮未读取或改变用户此设置，不能宣称已关闭。

Codex `forced_login_method = "chatgpt"` 限制登录方法，不是禁止消费额外 credits 的开关。官方说明套餐用完后可能消费已有余额；取消自动充值也不等于禁止已有余额被扣。详见[Codex 定价与额度](https://developers.openai.com/codex/pricing)及[个人套餐 credits](https://help.openai.com/en/articles/12642688)。尚未核实到可对本任务强制禁用 credits 的 CLI 开关。没有验证严格套餐内边界前，不发起订阅测试请求。

## 后续实现要求

1. 使用官方未修改的 CLI 和用户自己的官方订阅登录流程；不能让 Studiolo 代收订阅 OAuth 并把它注入通用 API relay。
2. 保留现有冻结、来源分类、fence/lease、受控工具、取消、候选人工落地与秘密隔离要求。需要先明确可信模型客户端与不可信工具执行进程的边界；不能默默给当前 Runner 挂载账号登录态或开放出网。
3. 新的认证、模型流量与工具控制必须分别验证。将官方 CLI 放在可信侧、工具放在隔离侧是待论证的方向，尚非已实现或已证明兼容的方案。
4. 套餐额度耗尽时停止并等待重置，不自动使用 API 或额外付费。共享订阅的并发消耗和账号付费设置必须纳入验证，轮询余额不能冒充硬计费上限。

当前结论：用户的计费限制已经记录，旧 API 配置入口已停用。订阅执行链需要后续适配，真实模型执行继续保持关闭；不得把普通 CLI 订阅登录成功写成完整 M6 验收。
