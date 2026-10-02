# MirrorCAS 与双向 Bridge 执行器

T-048，2026-09-17。源码在 `studiolo-sync-bridge` 的 `executor` 模块，38 项契约通过。
这是纯顺序核：它不打开文件、不连数据库、不发网络请求，也不自己判断受控镜像是否静止。
本页不代表真实 OwnedRun、真实 Broker/镜像往返或两周 headless 试跑已完成。

## 边界与端口

执行器只做一件事：**把一个已被持久层预留的冻结计划，按不可颠倒的顺序推到终态**。
全部外部能力都是 trait，实现方分别是受控镜像 owner、持久 owner、可信观察 adapter：

| 端口 | 职责 | 生产实现 |
|---|---|---|
| `MirrorQuiescence` | 给出受控镜像静止证据 | B3 `SyncTransportOwner` 的 `QuiescentMirror` |
| `PlanJournal` | 预留、逐动作状态、审计推进、退休与闭合 | 既有 `BridgeState`（私有 SQLite） |
| `ExecutionWitness` | 执行前重验与收尾完整共同观察 | 既有本地观察内核 |
| `MirrorContent` / `MirrorObjectSink` | 冻结版本正文的读出与受控镜像落库 | 受控镜像 owner |
| `JobBackend` | 按原幂等键执行一条已 Issued 动作 | Broker / 受控镜像安装器 |

纯规划器仍然没有任何权限；执行器也没有。`executor` 目录下不出现文件系统、
数据库或进程能力，架构守卫对 `crates/sync-bridge/src` 的既有禁令不需要为它开任何豁免。

## 固定顺序

1. 读持久快照；熔断、已退休、与冻结计划不是同一批，一律立刻返回。
2. **先算整批删除**：独立重数两个方向的删除、原共同基线存活路径与复制字节，
   与冻结统计逐字段对账，再判门槛。任何删除都在这一步之后才可能出现。
3. 核对证据：两侧观察必须完整且逐项显式；每条删除都必须「源侧显式 Missing ＋
   原共同基线存活」。冲突路径上不允许存在任何动作。
4. 取静止证据。取不到就不搬运；本批含删除时错误明确点名「删除缺静止」。
5. 推进 Intent 审计到真实 ACK，然后逐动作：**保全 → Issued → 执行 → 保存回执**。
6. 推进 Outcome 与整批 Summary 审计，再闭合计划。

## 静止只认一种证据

`QuiescenceEvidence` 是闭集，只有 `ControlledSubgroupIdle { spawn_gate_closed: true }`
被接受；`Unsupported` 与 `Unknown` 各自返回明确失败，绝不降级。
证据还必须绑定同一 `link_id`/`scope_epoch`/镜像根，镜像观察序号不得早于本批冻结观察，
且窗口未过期；实际可用窗口取「证据窗口」与「本批请求窗口」的较早者。
核验通过才铸出 `Quiesced` 票据，它没有公开构造函数，执行器无法自造。

## 保守删除熔断

阈值沿用主线冻结默认，**本片不提供任何解除入口**：

- 整批删除数 **> 5** 即阻断整个计划；**等号可过**。
- 整批删除数 **> 原完整共同基线存活路径数的 10%** 即阻断整个计划；**等号可过**。
- 分母是原共同基线里 canonical 存活的路径数；新建文件、已共同 Missing 的路径不扩大分母。
- 阈值判定复用纯规划器同一个函数，执行侧不存在第二套阈值实现。
- 所有计数走 checked 运算，乘法或加法溢出一律拒绝，绝不回绕。
- 存活路径数为 0 时任何删除都会触发比例门槛。
- 门槛未过时返回的是整批错误，没有「先放行前半批」的分支。

## MirrorCAS

镜像侧对象按内容寻址（SHA-256）落库，账本只记地址、字节与引用，真实字节归 sink。
**下列上限是本实现的内部工程上限，不是 Obsidian Sync Plus 的服务承诺，
也不能据此对用户宣称任何配额：**

- 单对象 256 MiB；
- stage 合计 512 MiB，**孤儿一并计入**；
- stage 条目 4096 项，**孤儿一并计入**；
- 同时接收中最多 1 个，同时 Prepared 最多 1 个；
- 固定帧 64 KiB，超过即拒。

声明字节数与实际字节数不符、实际内容不等于预期地址，都在封口阶段拒绝并让 sink 丢弃，
账本里不会留下半截对象。已在账本里的地址重复落库不占新条目。

用法与保全语义：

- 镜像侧复制（CreateOnly/Replace）先把源正文按地址落库，安装确认后交还额度。
- 镜像侧删除先把**目标当前字节**按地址落库，作为可恢复删除记录，之后才执行删除；
  该对象在执行器里保持被引用，不会被当成可回收孤儿。
  **保留期限（契约要求至少 7 天）由镜像 owner 负责，本片不承诺也不实现保留期。**
- Canonical 侧动作不进 MirrorCAS：canonical 的保全与可恢复删除记录归 Broker。

## 未知、失败与基线

- 后端拿不到权威终态时只能返回 `Unknown`。执行器既不保存回执，也不重发，
  更不会因为「目标当前内容碰巧相同」把它补成成功；本批不闭合。
- 重开后仍是 Issued 的动作按原键报告未知，**不重放**；解析它需要按原键查询后端回执，
  该入口属于后续 owner。
- 有明确无效果回执或本批仍有冲突时，只终结计划并保留旧共同基线。
- 只有全部终态已知、无失败、无冲突，且拿到属于当前打开实例的完整共同新观察，
  才带着共同观察闭合，共同基线是否真的前进由持久层前后快照比较得出，执行器不自报。
- 上一份完整终态还没有清理票据时，执行器先取票并推到真实审计 ACK，再闭合本计划；
  退休之后旧序号永久拒绝，**拒绝不代表它曾经成功**。

## 作业键

作业键来自持久层按单调计划序号分配的原幂等键，执行器只转发、不重算、不改写。
同一批里两条内容完全相同的动作也各有自己的键；内容 hash 不是作业键。
重试沿用原键，重开之后仍是原键。

## 验证与后续验收

38 项契约（行为 6、MirrorCAS 边界 8、失败 17、恢复 6，其中删除门槛 4 项单列）
在 macOS 通过，覆盖：内容寻址与去重、固定 64 KiB 帧、单对象与 stage 上限、
唯一接收/Prepared 槽、地址不符与截断、删除数 5 可过 6 阻断、恰好 10% 可过与超线阻断、
存活路径数为 0、整数溢出、无静止证据请求删除、未知/不支持/spawn 入口未关一律不降级、
观察不完整、冲突路径带动作、冻结统计对不上、审计 ACK 丢失与审计推不动、
持久熔断与退休永久拒绝、重开后原键重放且未知仍未知、未知不回收暂存额度、
明确无效果不推进基线、中途失败不留半截对象。

其中行为与恢复类用真实临时 SQLite 的 `BridgeState` 当持久层，证明端口可直接由它实现；
搬运、静止与审计身份仍是假实现。**本地契约不能证明真实手机往返、真实受控镜像静止、
冲突字节保全或两周 headless 试跑。** 生产装配还需要：把 `PlanJournal` 适配器接进组合根、
接入 B3 的真实 `QuiescentMirror`、由镜像 owner 实现 7 天保留与孤儿回收。

## 生产持久层（`sqlite::StateJournal`）

`PlanJournal` 的生产实现落在 `crates/sync-bridge/src/sqlite/executor_port.rs`，
经 `crate::sqlite::StateJournal` 公开。它是**唯一**的 BridgeState 适配实现：
测试夹具 `tests/executor_private/adapter.rs` 只在其外面包一层顺序日志（`LoggingJournal`），
不再复制任何映射逻辑。

三个协作者全部是显式构造参数，本类型没有隐式来源，也不持有文件系统、网络或进程能力：

```rust
StateJournal::new(state: &mut BridgeState, ledger: &dyn AuditLedger, input: Input)
```

- `state`：已打开并已核验绑定的持久 owner，真实落盘全部由它完成。
- `ledger`：受信任的权威审计 adapter；真实 append 在 SQL 事务之外，
  拿到精确回执之后才 CAS 保存 ACK，顺序由 `BridgeState::flush_audit` 保证。
- `input`：本批冻结原输入。它必须与持久层里冻结的那一份逐字段相同，
  否则持久层直接拒绝；适配器不做任何补正，也不允许执行中途替换（只读暴露 `frozen_input()`）。

### 失败映射

`BridgeError` → `JournalFault` 用**穷尽 match**写成，没有 catch-all：
持久层新增错误变体会让编译失败，而不是被悄悄压成某个已知类别。
执行器端没有对应类别的失败一律落在 `JournalFault::Unavailable`
——它的语义就是「拿不到权威结论」，上层永远不会把它折叠成成功。
落在这一类的是：`MissingStore` `AlreadyExists` `UnsafeStorage` `Busy`
`UnsupportedPlatform` `UnsupportedSchema` `CorruptStore` `Unavailable`
`Overflow` `PendingUnresolved` `RetryMismatch` `OutcomeUnknown`。
其余 12 个变体一一对应同名 `JournalFault`。

### 不放宽的边界

- `JobOutcome::Unknown` 在 `record_outcome` 里被显式拒绝（`OutcomeMismatch`），
  原键停在 Issued 等下次按原键核对，绝不写成回执、也不倒回 Reserved。
- 回执里的前置 revision 只取自冻结计划的 `target_expected`，不采信后端自报。
- 退休之后旧序号永久拒绝：整批面仍可读，逐动作面永久为空。
- 熔断在快照里显式可见，`settle` 与 `prepare_retirement` 都被永久挡住。

### 架构守卫

本文件不新增任何 fs 写，全部经由已登记的固定私有 owner，
因此**不需要新增豁免**：`sync-bridge` 的写能力豁免仍只有
`sqlite/storage.rs` 与 `sqlite/opening.rs` 两处。
它放在 `sqlite/` 路径下，也正是守卫允许引用 `sqlite::` 与 `studiolo_audit_ledger` 的位置。

### 验证

13 项契约在 `crates/sync-bridge/tests/sqlite_private/executor_port{,_failure,_recovery}.rs`，
全部跑真实临时 SQLite：行为 3、失败 7、恢复 3，覆盖正常推进与基线前进、
明确无效果保留旧基线、`BridgeError` 每个变体的映射、Intent 前置、未知终态被拒、
审计不可用与越界预算、错键回执、租约冲突、持久熔断、重开后原键重放与跨实例观察作废、
退休票据需要自己的真实审计 ACK、退休后永久拒绝。

冻结动作的源或目标发生变化时，执行器先取得真实新鲜双侧观察。BridgeState 仅对 Intent 已 ACK、从未发出且无回执的预约原子记录 `CancelledBeforeInstall`，不会返回 `IssuedJob`、调用文件写后端或保全陈旧源。持久 `issued` 位与取消票据同事务写入，表示原预约已消费，旧版读取仍为 Known；`bridge-unissued-cancel-<request digest>` 明确标识由持久 owner 取消。已发未知动作绝不走此入口。NoEffect 仍需真实 Outcome/Summary ACK，旧共同基线不推进，下一轮按现状重新规划。
