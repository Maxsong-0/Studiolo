# Runner Adapter Interface 契约

> Module：`studiolo-runner-adapter`。M0 的 `StubRunner` 与未来本地/VM Runner 共用同步生命周期
> Seam；Stub 只证明临时产物、幂等与取消，不预建真实 CLI/模型能力。

## 1. M0 Interface

```rust
pub trait RunnerAdapter: Send + Sync {
    fn probe(&self) -> Result<AdapterAvailability, RunnerError>;
    fn start(&self, spec: &RunSpec) -> Result<StartedRun, RunnerError>;
    fn observe(&self, handle: &RunHandle) -> Result<RunnerObservation, RunnerError>;
    fn cancel(&self, handle: &RunHandle) -> Result<(), RunnerError>;
}
```

- `AdapterAvailability` 是 `Ready | Unavailable { reason }`，不能从个人交互登录态推断 Ready。
- `RunSpec` 精确绑定 task、attempt、fence、人写 instruction 与 `Vec<RunInput>`；每个 input
  只有逻辑 `VaultPath`、捕获 revision 字符串与 snapshot 字节，不含 ambient Vault 路径。
- `StartedRun` 返回不透明 `RunHandle` 与可持久化 `RunnerRef`。
- `RunnerObservation` 是 `Running | Completed(RunnerReport) | Cancelled`；完成报告回显
  task/attempt/fence 与候选 `RunnerArtifact { name, content }`。

## 2. 生命周期与不变量

- TaskEngine 是 lifecycle owner；Adapter 不改任务状态、Control DB、canonical Vault 或审计。
- `start` 的幂等身份是 `(attempt_id, fence)`。完全相同 `RunSpec` 返回首次 `StartedRun`；同 key
  不同 spec 返回 `IdempotencyConflict`。
- `RunHandle` 只有创建它的 Adapter 能解释；未知 handle 明确报错。
- `cancel` 对既有 handle 幂等，先使后续 observe 为 Cancelled；重复取消仍成功。
- `RunnerRef::process(adapter, pid, start_time_ticks)` 要求 pid 与启动身份都非零，避免 PID
  复用误杀；Stub 使用无进程的 opaque reference。
- `Completed` 只是候选报告。TaskEngine 仍须验证当前 fence、产物和提交计划，进入持久
  `committing` 后再由 VaultBroker manifest-last；stdout/退出码不能绕过此顺序。

## 3. M0 Stub

`StubRunner` 为每个新 `(attempt, fence)` 创建私有临时目录，在其中同步生成并 fsync
`result.md`。内容由 task、attempt、instruction 以及每个 input 的逻辑 path/revision/bytes
派生；`observe` 再把候选字节复制到结构化报告，调用方看不到临时目录句柄。

Stub 不持有 canonical Vault、Control DB、history、audit、宿主 home 或凭据路径。唯一持久化
通路仍是 TaskEngine → VaultBroker。M0 Stub 同步完成，所以本实现不主动产生 Running；枚举
保留该状态供未来 Adapter 和测试替身。

失败、阻塞、崩溃、超时、分页事件 cursor、Gateway endpoint/capability 与强制进程回收不在
当前 Stub 实现内。TaskEngine 的竞态/reconcile 测试应注入实现同一 trait 的 spy/failing
Adapter；不能把这些场景冒充 Stub 已实现，也不能在 reconcile 中调用 `start`。

## 4. M0 错误模式

| `RunnerError` | 含义 |
|---|---|
| `UnknownHandle` | handle 不属于此 Adapter 实例 |
| `IdempotencyConflict` | 同 attempt/fence 绑定不同不可变输入 |
| `InvalidRunnerRef` | process ref 的 pid/启动身份为零 |
| `StateUnavailable` | Adapter 内存状态锁不可用 |
| `Io { operation, source }` | 私有临时存储创建、写、sync 或读失败 |

`Unavailable` 是 probe 的结构化结果而不是 panic。未来进程协议错误、超时、输出越界等必须
扩展类型化错误/终态，不能复用 `Io` 隐藏语义。

## 5. Adapter 与验收

契约测试至少覆盖：trait object 可用、probe 显式 Ready、同 spec start 重放、同 key 异 spec
冲突、输出包含输入证据且只以候选字节返回、重复 cancel、取消后 observe、未知 handle、
process ref 防 PID 复用，以及源码/架构测试证明本 crate 没有 canonical Vault 写入口。

重启 reconcile 另用 spy Adapter 统计 `start` 调用次数：销毁并重建 TaskEngine 后计数不得增加。
这项通过不要求 Stub 恢复其内存 handle，而是要求 TaskEngine 只前滚旧运行事实。
