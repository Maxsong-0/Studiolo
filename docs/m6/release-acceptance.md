# M6 历史服务器隔离验收模板

> 当前发布使用本机 Codex / Claude Code 的订阅额度派发，真实结果见 [本机接入验收](local-client-dispatch.md)。本文保留历史服务器 Runner 隔离检查清单，不是当前启用指令；禁止按此页准备 API 凭据或启动模型 key-holder。

本表是实际验收时填写的记录，不是成功回执。代码和 fixture 通过不自动勾选任何部署项。

## 已由发起人决定

- Claude、Codex 可写 `项目/Studiolo` 黑板；其余已登记类型只读，未登记类型零权限。
- 最新决定：只用本人本机原生客户端的订阅额度，禁止模型 API 和额外付费回退。原 Gateway 专用 API 凭据决定已被撤销。
- githubci 仅完成历史隔离探针；当前本机订阅链路已真实验证。以下服务器 Runner 模型与发布门槛不属于本次启用范围，也未被本机验收替代。

## 需绑定的部署信息

| 项目 | 实际值 |
|---|---|
| 主机、操作系统、内核 | 待提供 |
| 本次 release commit / 二进制摘要 | 待构建发布 |
| canonical 服务账号 / Gateway 服务账号 UID | 待提供，必须不同 |
| 两家原生 CLI 版本 / SHA-256 / 最小 rootfs | 待安装并记录 |
| 私有 scratch / retained / raw / socket 路径 | 待配置 |
| Gateway 凭据文件路径（不记录内容） | 待配置 |
| 当前 policy effective hash / 来源 revision 与 SHA-256 | 待实际读取 |
| 专用模型 / token、费用、时间上限 | 待配置并经人侧确认 |
| 数据外发、产物 backup / desktop_sync 范围 | 待按实际来源分类 |

## 二元验收

1. 用真实服务身份运行两家 `probe`，记录 `binary_version`、`protocol_capabilities`、availability 和缺失项。缺项必须 unavailable，禁止复用旧 Ready。
2. 每家一个最小真实任务：Gateway-only 专用密钥，固定模型/endpoint，CLI 只持短期 attempt/fence capability。证明根文件系统/个人登录目录/Vault/Control DB/权威审计不可达，直接外网、私网及 metadata 网络不可达。
3. 完整调用：审计 intent → 硬预算预留 → 上游 → raw 私有持久化 → Broker metadata seal → outcome → 释放响应。重复 call ID 不重复外呼；未知上游结果禁止自动重放。
4. 调用中依次撤销实例、祖先、类型、策略、租约、fence 及全局冻结，后续流或调用均拒绝。分别触达工具/费用/token/时长上限，不能继续支出。
5. 取消后先撤销 Gateway 权限，180 秒窗口内有效 stopped 回执与 patch 保留；无有效回执窗口到期 abandoned。确认完整 cgroup 为空，无遗留子进程；日志不能把隔离等同验收成功。
6. 零工具启动失败最多再试两次；首个工具后失败或协议/输出损坏不按 launch_failure 重试；十分钟无真实进度对账 abandoned。
7. CLI 成功只到 NeedsReview。候选含 base/after tree 与 SHA-256；修改 HEAD/脏工作树后落地拒绝；冲突保留。L3 明确批准后使用仓库锁落地，真实 index 和 refs 不被自动提交/推送；再次核验后才人工逐项完成回执。
8. 各阶段重启：先隔离旧作用域，T1 前滚，旧 capability 失效，候选不丢。仅已知终态、确认隔离、足龄且身份未变的注册 worktree 可清理，先保留 patch；未知目录和冲突不删除。
9. 真实 Sync 往返及备份范围单独验收：TASKS 人写值保留、生成区带外改动迁移，raw/长期凭据不进入镜像；受控产物只有在明确批准的范围内进入备份和同步。

每项记录：时间、服务身份、release、最小操作、期望/实际、审计 event/offset、保留产物指针、pass/fail/unverified。失败保持冻结，并明确是否有已提交状态或需人工恢复的候选。

Demo 初始化 CLI 已提供（见 `stub-gateway.md`）：创建全新演示库、Web/DB、持久审计与一次候选闭环，可保持 Web 运行。它不注册生产服务；GUI 安装入口另需产品验收。两家真实 CLI 仍不能由 Demo 结果替代。
