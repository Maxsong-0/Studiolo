# M5 黑板组合与首次权限

2026-09-20，主人确认：Claude、Codex 可写 `项目/Studiolo` 黑板，其他已登记 AI 保持只读。未登记类型仍是零权限。本文及示例不代表线上策略已激活。

`write.board=true` 必须同时声明 `write.board_projects`，后者只能是读取白名单内的完整项目路径。扩大读取范围不会扩大写入范围；`content_direct` 永久为 false，正文必须走预览、CAS patch 和批次事务。`initial-capabilities/` 是待发布示例：使用时合并当前读取白名单及 hard_bound_skills，不能用示例覆盖既有配置。实际发布仍经过现有 desired → diff → One-time Code → effective。

## 显式装配

local-server 在既有 canonical writer、应用会话、权威审计、策略库上，额外接受：

```text
--board-socket /run/studiolo-board/board.sock
--board-data /var/lib/studiolo-board
--board-agent-data /var/lib/studiolo-agent
--policy-data /var/lib/studiolo-policy
```

路径仅示意，必须按实际服务身份与既有宿主配置设置。控制目录是独立 0700、同 UID 目录，固定 `control.sqlite3` 为 0600；不得与 Vault、Broker、身份、策略、认证、审计、静态资源或宿主配置目录重叠。私有 Unix socket 只接受同 UID 的可信连接，不加入公网路由。该组合借用原 canonical 单写者，不打开第二个 Broker。

agent-server 的 `serve --board-socket <相同socket> --policy <既有策略目录>` 增加 `board` 工具。调用方只提交 `project`、`operation`、结构化 `request`；类型和实例由已认证 MCP 会话注入。每次调用在转发端与 canonical 端核验实例链、项目和现行策略，失败或服务不可用即停止 A2A，不用文件副本代替实时黑板。

子实例需要先显式运行身份库管理命令 `identity-upgrade`（参数见 `studiolo-agent --help`）。随后 MCP initialize 可通过 `_meta["studiolo.parent_instance"]` 声明同类型父实例。父实例或祖先吊销立即拒绝后代调用。原 v0 身份库不隐式迁移。

## 操作契约

- `list/get`：白名单内只读。
- `create/claim/heartbeat/handoff/return/cancel/number`：显式写项目授权，结构和租约受 TaskEngine 校验，参见 [黑板契约](blackboard.md)。
- `batch.preview/batch.commit/batch.rollback`：当前有效 claim、项目与 `files_owned` 三者交集；批次 ID 按实例和任务命名，预览哈希与全部 revision 一起验证。
- `memory.preview`：只在本项目 `03-记忆` 下生成结构化条目；重复返回已有条目指针，仍需显式 patch。通用批次入口也执行记忆约束，不能绕过查重。

人侧 `GET /api/v1/board?project=项目/Studiolo` 是控制状态投影；`GET /api/v1/board/replay?task_id=…&after=…` 每页至多 100 条权威审计元数据。两者都在原应用会话门禁内。图 1 展示树、持有/历史状态、验收与指针；图 3 仅提供允许的审计字段，绝不下发原始模型响应。

黑板权威状态提交后，TaskEngine 通过批量写设施投影 `04-协调/TASKS.md`。生成区外人写内容保留；人手改动生成区被迁移保留并记审计。投影失败会明确报“权威状态已提交，投影待恢复”，维护循环继续重试，不把它冒充未发生。

## 执行体人侧控制

可选 `--runner-config <私有0600配置>` 必须同时配置 board 与真实因子 Unix 入口；缺配置时执行页返回 `configured=false,frozen=true`。Gateway 长期凭据由独立 UID 进程持有，详见 [M6 装配](../m6/integration.md)。

`POST /api/v1/execution/command` 仅接受固定操作：`freeze`、`start`、`cancel`、`land`、`review`。全部重验当前应用会话与 CSRF、写入意图/结果审计；解除冻结、代码落地与候选验收额外要求当前 L3。AI 的 board 工具没有这些人侧入口。`review` 必须带完整 `BoardReceipt`、真实候选指针、逐条验收和 closeout；CLI 退出成功只产生 `NeedsReview`，不自动完成任务。

只读状态 `GET /api/v1/execution` 与黑板一起表达执行视图。`candidate_retained` 表示本地候选已保留但尚未成功封存，不能从该状态伪造验收。原始响应、长期密钥、短期 bearer 和候选正文均不进入 Control DB 或这些监督 JSON。

## 发布边界

仓库验证只证明合成库和本地契约；线上策略发布、身份迁移、服务启动、真实 Sync 往返、两种 CLI 的专用 API 验收和 RG-RUNNER 负向清单各需单独证据。开发完成不能替代这些发布门槛。

人侧黑板投影追加 `instance_lineages`，只返回已登记的同类型父链元数据。项目时间回放 `/api/v1/board/timeline` 使用当前会话，按项目与最多31天窗口连接有界审计页及新鲜日志指针；全局扫描游标即使空页也前进，不含审计 payload 或日志正文，不伪造历史状态快照。
