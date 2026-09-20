# M6 取消产物与 stopped 回执

可信组合根在已收到取消信号后，采集 Runner 的真实 patch 与结构化回执，调用：

```rust
pub fn stage_board_stopped_candidates(
    &mut self,
    report: RunnerReport,
    receipt: BoardReceipt,
    now_ms: i64,
) -> Result<ArtifactCommitReceipt, TaskEngineError>
```

这不是 AI board `return` 的入口。非 consumer 的普通 `return` 不能提交 completed 或 stopped；普通 `report` 不接受 board 任务，普通 candidate 入口不能把 CancelRequested 变成 NeedsReview。没有有效 stopped 回执时，组合根不得调用 acknowledge_cancel；任务等待取消窗口到期后成为 Abandoned。

## 准入与真实材料

- task、attempt、fence 必须属于同一当前或最近 attempt；首次 T1 还必须是当前 CancelRequested，且 `cancel_requested_ms <= now < cancel_requested_ms + 180000`。取消已即时撤销 Gateway 权限；此窗口不重新授权任何工具，也不续租。
- 只接受 `status: stopped`。逐项验收和 closeout 必填，冲突列表必须为空；保留原始 Fail/Unverified，允许有 canonical evidence 的部分 Pass，但至少一项未完成。引擎不生成 Pass，也不将停止解释成完成。
- Report 必须含真实 `candidate.patch` 与 `patch.json`。沿用正常候选的 SHA-256、Git tree id、冲突标记验证；可多带一份 `stopped.json`，其反序列化值必须与传入 BoardReceipt 完全相等。若未带，则将同一回执序列化后追加，绝不补造验收字段。
- 输入 artifacts/evidence/unlanded 如有引用，须经 Broker 读取核对真实 revision/hash。原始回执仍保存为独立 stopped.json；候选 patch 留在内容面，并未落到被修改的源码目标。
- 原始回执序列化限 32 KiB，artifacts 至多 253、unlanded 至多 255，为系统追加的三个产物和 patch 指针预留界限。

## 复用 T1/T2

1. Broker staging 耐久保存原始候选与 stopped.json。`runner_candidate_stages` 只登记 `{id, sha256}`。拒绝或过窗也保留已登记指针；reconcile 仍可将未取得 T1 的过窗任务标记为 Abandoned。
2. 同一 ControlDb 事务内，`reserve_board_stopped_candidates` 验证取消窗口、当前归属、子项终态，并将既有 attempt/task 切为 Committing。`terminal_json` 只存 format、event_id、task_id、attempt_id、fence、可信 `outcome: stopped` 以及 staging 指针，不存 patch、原始回执或计划正文。
3. 原有 `roll_forward` 加载并校验 staging，写审计 Intent，使用相同 artifact commit 幂等键，manifest 最后发布。
4. 读取校验实际 committed artifacts 后，T2 一次事务写 artifact_commits、结构化 stopped 回执、Cancelled、last_attempt、清除活跃占用，以及终态审计/投影 outbox。回执追加全部真实 artifact 指针，并把 candidate.patch 的 canonical 内容面指针加入 unlanded；原始验收结论不变。
5. T1 后发生崩溃，启动/周期恢复只继续原提交；即使已过 180 秒也不丢弃已取得 T1 的赢家，不再次运行 CLI。同参数重放返回原 commit receipt；改变 patch 或 stopped 包的重放拒绝。

取消与正常候选复用同一提交状态机，没有新增生命周期表或 schema 版本。v1/v2 历史内联 reservation 仍先经 Broker staging 再 CAS 移除正文；旧记录无 `outcome`，不能在迁移时变成 stopped。最终状态不得与 T1 冻结的 outcome 不一致。

## 验收边界

`crates/task-engine/tests/stopped_candidates_contract.rs` 覆盖真实产物/回执、即时 Gateway guard 撤销、普通 AI 绕过/旧 fence/伪造 packet/虚假全 Pass 拒绝、截止边界 retained→Abandoned、真实 SQLite/FsBroker manifest 确认丢失→跨窗口恢复、无 T1 正文及幂等重放。这里不证明真实 CLI 已按约停止；Runner stdout 采集和组合根接线另由其集成测试验证。

本轮子任务未运行编译或测试。主线程统一运行 candidate/board/stopped 契约与 workspace clippy。
