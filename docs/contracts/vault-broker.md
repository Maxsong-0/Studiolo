# VaultBroker Interface 契约

> Module：`studiolo-vault-broker`。它是 canonical Vault 与共享 L0 history 的唯一写者，
> 集中路径安全、CAS、幂等、快照一致性、原子落盘和 attempt commit point。

## 1. M0 Interface

M0 按调用方最小权限拆成四个能力 trait，再由 `VaultBroker` 组合：

```rust
pub trait VaultReader: Send + Sync {
    fn vault_uuid(&self) -> VaultUuid;
    fn read(&self, path: &VaultPath) -> Result<VaultObject, BrokerError>;
}

pub trait VaultSnapshotter: Send + Sync {
    fn snapshot(&self, inputs: &[SnapshotInput]) -> Result<Snapshot, BrokerError>;
}

pub trait SnapshotReader: Send + Sync {
    fn load_snapshot(&self, id: &SnapshotId) -> Result<Snapshot, BrokerError>;
}

pub trait VaultWriter: Send + Sync {
    fn mutate(&mut self, mutation: Mutation) -> Result<MutationReceipt, BrokerError>;
    fn commit_attempt_artifacts(
        &mut self,
        commit: ArtifactCommit,
    ) -> Result<ArtifactCommitReceipt, BrokerError>;
}

pub trait VaultBroker:
    VaultReader + VaultSnapshotter + SnapshotReader + VaultWriter {}
```

M0 不暴露 `changes(cursor)`：没有 watcher/cursor 时伪造变更流会制造错误的新鲜度保证。
增量 change feed 延期到确有 M1 消费者时设计；当前没有该方法或对应的假成功返回值。

## 2. DTO 与责任

- `VaultObject` 返回逻辑路径、完整字节和绑定 Vault/path/content 的 `Revision`。
- `SnapshotInput` 是 path + expected revision；`Snapshot` 含不透明 `SnapshotId`、Vault UUID
  与按 path 排序的只读 `SnapshotFile`。
- `Mutation` 只支持 create-only 或带 expected revision 的完整替换；`Precondition` 为
  `MustNotExist | Match(Revision)`，并携带稳定 idempotency key。
- `ArtifactCommit` 绑定 task、attempt、fence、审计 intent correlation、幂等键及候选产物；
  Broker 派生所有 canonical 目标，调用方不能自选 manifest 路径。
- `ArtifactCommitReceipt` 返回 manifest 路径/revision 及每个产物的 path/revision/SHA-256。

调用方只传 `VaultPath` 与值对象，不能传环境绝对路径或任意已打开的文件描述符。
`cap-std::Dir`、临时文件名和 history 布局属于实现细节。

## 3. 路径与对象安全

所有读、写、快照、历史与 attempt 提交从部署时批准的 Vault 根目录句柄出发。字符串校验
之后仍须：

- 逐分量拒绝 symlink；Linux 使用 `openat2` 的 beneath/no-magiclinks/no-symlinks 约束。
  当前 Linux Adapter 不做弱化回退，内核不支持时失败关闭；macOS 开发回路使用 cap-std；
- 打开后校验允许的普通文件/目录，普通文件 link count 必须为一；
- 拒绝 magic link、hardlink、设备、FIFO、socket、编码分隔符、大小写/Unicode 碰撞及
  校验后竞态替换；
- 普通 mutation 拒绝 `.git/**`、`.workbench/**`、`.obsidian/**` 与 `.stignore`。

`.workbench/vault.json` 仅由 `FsVaultBroker::open` 的显式初始化路径创建；既有打开及
canonical窄读只验证已有身份。至少绑定格式版本与Vault UUID；损坏时失败关闭，
普通mutation永远不能修改它。写者的不可变配对和根身份约束见[T-028契约](canonical-writer.md)。

## 4. revision、snapshot 与普通 mutation

- revision 使用 [core-types.md](core-types.md) 的域分离 SHA-256，不以 mtime/size 代替。
- `read` 必须拒绝 link/特殊文件并检测不稳定读取；不能返回撕裂字节的 revision。
- `snapshot` 先验证每个 expected revision，按逻辑路径稳定排序并生成内容寻址 ID；任一输入
  不符返回 `RevisionConflict`，捕获后再复核所有源 revision，变化则整体 `UnstableRead`。
- `SnapshotId` 只接受 64 位小写十六进制文本；`load_snapshot` 校验请求 ID、Vault UUID、
  路径严格排序/唯一、每项内容 revision 与整体 ID。持久记录失配返回 `CorruptSnapshot`，
  不能把可反序列化的 JSON 直接视为可信输入。
- 普通替换先在 data root 的内容寻址 L0 history 保存旧 revision，再以同文件系统临时文件
  + rename 形成原子可见提交；失败不能留下半截 canonical 文件。
- Fs Adapter 串行读取并检查打开前后文件身份、类型、nlink 与时间/大小；Linux 以
  NONBLOCK 打开避免竞态替换成 FIFO 后挂起。rename 前再核 expected revision 和 pinned
  parent 身份；当前mutation的create-only在Linux/macOS使用无覆盖rename安装。
- 幂等键与规范化请求指纹绑定：已完成同请求返回第一次receipt；明确CAS拒绝保持拒绝，
  同key异请求返回`IdempotencyConflict`。Installing未知结果保持未决，不借当前相同内容补成功。

Fs写者持有canonical根目录FD的跨进程独占租约；永久root/data配对防止换数据目录绕过。
替换式 CAS 的 revision 检查与操作系统 rename 仍是两个
调用，不能对完全不遵守 Broker 锁的外部进程声称严格线性化 CAS；实现拒绝已观察到的变化
并校验提交后结果。正式 bridge 必须继续遵循 ADR-005，经 Broker 摄入 canonical Vault。

## 5. attempt manifest-last

固定且唯一的 commit point 是：

```text
sources/attempts/<task-id>/<attempt-id>/manifest.json
```

Broker 校验 task/attempt/fence、审计 intent correlation、候选文件类型与内容摘要，按稳定
顺序写 artifact，并把 manifest 作为最后一个 canonical 写。manifest 的逻辑证明必须足以
关联 Vault 身份、task/attempt/fence、请求指纹、审计 intent 以及每个 artifact 的路径、
revision 和 SHA-256；M0 不冻结当前 private Rust struct 的 JSON 字段布局或未来公开格式。

manifest 前的文件是 orphan，不构成成功；manifest 出现后内容不可变。同 attempt 同请求重放
返回首次 receipt，不重复 history/产物；已存在内容不同则 `ArtifactConflict` 或
`InvalidArtifact`。VaultBroker 不自行把 TaskEngine 状态改为 `needs_review`。

重放已有 manifest 时仍验证格式、task/attempt/fence/审计关联、artifact 数量、全部派生目标、
hash/revision 和当前文件内容。manifest 存在但证明不成立时拒绝，不以文件存在替代完整性。

## 6. 错误模式（含M1/M2读写扩展）

| `BrokerError` | 含义 |
|---|---|
| `ProtectedPath(path)` | 普通 mutation 命中保护域 |
| `NotFound(path)` | 对象或 snapshot 不存在 |
| `RevisionConflict { path, expected, actual }` | CAS 前置不成立 |
| `UnsafeNode { path, reason }` | link、特殊文件或不安全目录分量 |
| `PathCollision { requested, existing }` | 大小写/规范化拼写碰撞 |
| `IdempotencyConflict` | 同 key 绑定不同请求 |
| `InvalidArtifact(reason)` | commit 身份、目标或内容计划非法 |
| `ArtifactConflict(path)` | orphan/既有产物与重放内容不同 |
| `CorruptIdentity` | Vault identity 损坏 |
| `WriterBusy` | canonical根已有存活独占写者 |
| `WriterBindingMismatch` | 根、data或不可变控制对象与配对不符 |
| `CorruptWriterState` | 持久日志/回执非法或违反阶段不变量 |
| `MutationUnresolved` | 安装或legacy旧操作未决，不能开始其他canonical写入 |
| `MutationCancelled` | 原请求已明确取消；重放不会再次尝试安装 |
| `MutationIndeterminate { evidence_id }` | 未知安装已封存证据并终止重放；既不证明原请求成功也不证明失败，`evidence_id` 只是私有协调证据 ID |
| `WriterLimitExceeded` | 输入或写协议存储超过固定预算 |
| `CaptureChanged` | 捕获所属实例或已确认提交水位变化，候选视图不能发布 |
| `IncompleteCapture` | 扫描出现安全诊断，部分结果不得当作完整捕获 |
| `InvalidHistoryCursor` | 历史游标语法错误或不属于本路径 |
| `HistoryChanged` | 历史目录已变化，调用方须重新从第一页开始 |
| `RootChanged` | 所选canonical根已改变或不可用，不能继续用旧路径身份 |
| `ReadLimitExceeded { path, max_bytes }` | 超过调用者指定的有界读取预算，包含读取期间增长 |
| `CorruptSnapshot` | snapshot ID、Vault 绑定、排序或内容证明不一致 |
| `UnstableRead(path)` | snapshot/read 期间内容变化 |
| `Io { operation, source }` | 不泄漏环境路径的文件系统失败 |
| `Json { operation, source }` | 内部 durable record 无法解析 |

语法非法的 `VaultPath` 在进入 Broker 前由 `PathSyntaxError` 拒绝。
2026-09-17 T-056 按 `crates/vault-broker/src/lib.rs:94-200` 逐项核对：上表 26 行即当前
`BrokerError` 的全部变体，`MutationCancelled` / `MutationIndeterminate` / `CaptureChanged` /
`IncompleteCapture` / `InvalidHistoryCursor` / `HistoryChanged` 六项是本轮补齐的既有变体。

M1 `ReadOnlyVault`及M2 `CanonicalReadVault`复用安全只读内核，不授予M0写能力；
后者只接受已有Vault身份并返回真实revision，见[canonical读取契约](canonical-read.md)。
T-028单写者与未决幂等已通过两端全量与生产检查，旧失败CAS已真实复现并作为回归修复，见
[写者契约](canonical-writer.md)与[设计输入](../m2/canonical-write-design.md)。
索引新鲜度、HTTP保存、流式附件和显式未决协调不由本片状态水位代替。

## 7. Adapter 与验收

`FsVaultBroker` 是 M0 真实 Adapter。调用方契约测试可使用受约束的内存替身；文件语义与
Linux 路径攻击必须通过临时目录上的真实 Adapter 验证，不能用内存替身代替。

验收至少覆盖：create/replace/CAS、history、幂等冲突、snapshot 稳定性、Vault UUID 初始化、
manifest 最后写与重放，以及 symlink、magic link、encoded separator、大小写/Unicode
碰撞、竞态替换、hardlink、设备/FIFO/socket。Linux 专属测试未实际运行时只能标“未验证”。
