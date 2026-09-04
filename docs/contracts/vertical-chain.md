# M0 最小垂直链与重启验收

> 目标：用临时 Vault、真实 Control DB、真实 Stub Runner 和 Module Seam 证明一条结构化
> 任务可安全提交；不启动 Web、真实 Gateway、真实模型或真实数据源。

## 1. 固定目录与 commit point

```text
<temp>/canonical/                                  # 测试 canonical Vault
<temp>/workbench-data/control.db                   # 同一文件用于重启测试
<temp>/workbench-data/history/
<private-temp>/                                    # Stub 自建的私有临时目录

canonical/ops/tasks/<task-id>.md                   # task 页面与受保护投影
canonical/sources/attempts/<task-id>/<attempt-id>/... # M0 已验证 artifact
canonical/sources/attempts/<task-id>/<attempt-id>/manifest.json
```

唯一 attempt commit point 是最后一行。attempt 目录中的文件即使已存在，只要 manifest
不存在就仍是 orphan，不可进入任务结论、快照可信输入或 `needs_review`。

manifest 的逻辑证明关联 Vault 身份、task/attempt/fence、请求指纹、审计 intent 与按路径
排序的 artifact path/revision/SHA-256。当前 private Rust struct 的 JSON 字段名、framing 与
未来公开 schema 不属于 M0 冻结项；验收从 `ArtifactCommitReceipt` 和可验证文件关系观察，
不能依赖 private 字段布局。

## 2. 正常时序

1. 初始化临时 canonical Vault：VaultBroker 创建 `.workbench/vault.json`，Control DB 绑定同一
   Vault UUID。
2. 通过 VaultBroker 创建一篇结构化 task；人写字段与投影字段按 core-types 契约分开。
3. 受信任进程内 `Run` command 为 task 页面和显式 input paths 创建 snapshot，并保存其
   metadata 后排队；同一 Queued task 重放 Run 不换输入。
4. M0 没有远程 AuthContext、policy 版本或独立 command 幂等键，不宣称实现完整 Dispatch。
5. Worker 加载 Run 冻结的 snapshot；`claim` transaction 创建 attempt、lease、单调 fence，
   TaskEngine 返回从该 snapshot 派生的 RunSpec。
6. Stub `start` 只拿含 snapshot 字节的不可变 `RunSpec`；Stub 自建私有临时目录，TaskEngine
   把 opaque runner ref 附到 Running attempt。
7. Stub 从输入生成结果文件和结构化 artifact declaration；`observe/report` 使用稳定 event ID。
8. TaskEngine 验证当前 fence，在单 transaction 进入 `committing` 并保存 report/commit plan；
   Broker 在 canonical 提交边界验证目标与内容摘要。
9. AuditLedger intent 成功；失败则不写 canonical artifact。
10. TaskEngine 再核对持久 Committing reservation/fence 后，VaultBroker 逐项幂等落 artifact，
    最后原子写固定 manifest；Broker 绑定该 fence，但 M0 不持有 Control DB guard。
11. TaskEngine 取得验证后的 receipt，在单 transaction 进入 `needs_review`，同时写 projection
    outbox 和 terminal audit outbox。
12. AuditLedger 从 durable outbox 写 outcome 后 acknowledge；投影器独立经 VaultBroker
    更新 task 的 `status/active_attempt/last_attempt/input_snapshot` 并确认投影交付。

可观察成功条件：Control DB 为 `needs_review`、active attempt 已清、last attempt 正确、manifest
存在且能验证全部 artifact、task 投影最终收敛、intent/outcome 可按同 event ID 关联。

## 3. 崩溃与 reconcile 矩阵

每个注入点都销毁 TaskEngine 实例，再以同一 `control.db` 和 canonical Vault 构造新实例。

| 崩溃点/启动事实 | 必须结果 |
|---|---|
| Run transaction 前 | 无 queued 状态；之前创建的 Draft 投影可仍待交付，Run 可安全重试 |
| claim transaction 前 | 无 attempt；正常 claim 可继续 |
| claim 已提交、Runner 未确认启动 | attempt 不复活；reconcile 使其不再活跃并 `abandoned` |
| `running` 任意时刻 | 不调用 Stub `start`；旧 attempt `abandoned`，迟到 report 为 stale/terminal |
| `cancel_requested` | M0 Stub 不会在重启后留存独立进程，收敛 Cancelled；不恢复为 Running |
| `committing` 已保存，尚无 artifact | 用 reservation 中保存的候选字节前滚提交，不依赖 Runner 临时目录；损坏计划失败关闭，不重跑 Stub |
| 部分 artifact 已落、无 manifest | 校验同 hash 后继续缺失项；异 hash 失败；orphan 不算成功 |
| manifest 已落、DB 尚未 finalize | 不复制 artifact；校验 manifest 后前滚 `needs_review` |
| DB 已 `needs_review`、outbox 未投影 | 只重试 outbox；不回滚状态、不重跑 Runner |
| T2 已完成、Audit outcome 未写 | 从 terminal audit outbox 按稳定 event ID 补 outcome；不重复调用 Broker 或 Runner |
| manifest 存在但 hash/fence/task 不符 | integrity failure，保持不可用并告警；不得采纳 |

M0 reconcile 不根据 retry policy 自动排队。未来允许 retry 时也只能新建 attempt/new fence。

## 4. 幂等与竞态用例

- 同一 Queued task 重放 Run 保留原 snapshot；M0 不提供跨终态重新排队的 Run。
- 同一 claim 并发竞争只生成一个 active attempt。
- 同 event ID/同 report 重放返回第一次结果；异 payload 拒绝。
- 同 CommitAttempt 重放不重复 history/artifact/manifest；同 attempt 异内容拒绝。
- 取消在 Runner 成功 transaction 前获胜时，迟到成功不能进入 committing。
- committing 已封存后，取消不得删除或改写 commit plan；按已持久化事实前滚并记录竞态。
- 旧 fence 的 Gateway、heartbeat、report 全部拒绝；TaskEngine 不得为旧 fence 构造 Vault
  commit，VaultBroker 只接受当前已封存 reservation 的请求。

## 5. 写入口架构验收

集成测试之外还必须静态断言：

- TaskEngine、Gateway、Runner Adapter 与 Studiolo 组合根没有 canonical Vault 直接写调用；
- 只有 VaultBroker 使用 Vault 根目录句柄和 history 写实现；
- 只有 TaskEngine 持有 ControlDb 写权限，control-db 内封装写 SQL；
- Gateway 不依赖 TaskEngine crate；
- Stub 的所有输出都位于 attempt temp，最终 artifact 由 VaultBroker 创建；
- manifest 路径在代码与本契约中只有一个权威构造函数。

## 6. 测试完成线

至少有三类独立测试：

1. `vertical_chain`：正常链、投影和 manifest 验证；
2. `restart_reconcile`：逐崩溃点复用同一数据库，spy Stub 断言无额外 start；
3. `write_entry_architecture`：依赖图和源码写入口断言。

测试名可按 Rust 约定调整，但缺少任一类行为覆盖不得宣称 M0 完成。测试只使用项目创建的
临时目录，不读写用户真实 Vault。
