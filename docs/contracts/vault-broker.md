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

`.workbench/vault.json` 仅由 `FsVaultBroker::open` 的初始化路径创建或读取；至少绑定格式
版本与 Vault UUID。存在但损坏时失败关闭，普通 mutation 永远不能修改它。

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
  parent 身份；create-only 在 Linux 使用 `RENAME_NOREPLACE`，macOS 使用原子 link 安装。
- 幂等键与规范化请求指纹绑定：同请求返回第一次 receipt，同 key 异请求
  `IdempotencyConflict`，不能覆盖旧结果。

进程内只允许一个 Broker 写者。替换式 CAS 的 revision 检查与操作系统 rename 仍是两个
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

## 6. M0 错误模式

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
| `CorruptSnapshot` | snapshot ID、Vault 绑定、排序或内容证明不一致 |
| `UnstableRead(path)` | snapshot/read 期间内容变化 |
| `Io { operation, source }` | 不泄漏环境路径的文件系统失败 |
| `Json { operation, source }` | 内部 durable record 无法解析 |

语法非法的 `VaultPath` 在进入 Broker 前由 `PathSyntaxError` 拒绝。

## 7. Adapter 与验收

`FsVaultBroker` 是 M0 真实 Adapter。调用方契约测试可使用受约束的内存替身；文件语义与
Linux 路径攻击必须通过临时目录上的真实 Adapter 验证，不能用内存替身代替。

验收至少覆盖：create/replace/CAS、history、幂等冲突、snapshot 稳定性、Vault UUID 初始化、
manifest 最后写与重放，以及 symlink、magic link、encoded separator、大小写/Unicode
碰撞、竞态替换、hardlink、设备/FIFO/socket。Linux 专属测试未实际运行时只能标“未验证”。
