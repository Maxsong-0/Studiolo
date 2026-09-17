# T-046：复合 Rename 实施契约

日期：2026-09-13；2026-09-14更新验收。Broker、服务端审计/维护、人侧策略、HTTP和界面已整合；人侧策略114项、前端687项及合成浏览器通过。1006构建输入冻结点的完整workspace在macOS145目标1719项、Linux aarch64145目标1731项通过；两平台fmt、严格Clippy、生产构建及独立目录17项进程契约通过。尚未在真实Vault启用或发布。
依据：`docs/m2/file-operations-design.md` 第 5—7 节；以下版本/API 已按登记实施，实际人侧身份和生产部署仍待验收，完整台账见 `m2-m3-closeout.md`。
仓库基准目录：`/Users/maxsong/All-Project/studiolo`；未注明 crate 的源码路径相对 `crates/vault-broker/src`，local-server 路径相对 `crates`。

## 1. 首片边界与公共入口

首片完成 Broker、固定审计映射及离线维护；不把两个旧 HTTP 操作串起来，也不先开放前端按钮。
沿用同一个 `FsVaultBroker`、`FileOperationMetadata`、`prepare_file_operation`、真实 Intent ACK、借用票据 commit、消费票据 cancel。
选定显式 `enable_file_rename(&mut self) -> Result<(), BrokerError>`：只有完整维护/审计适配已装配的可信 owner 才调用；普通 enable_file_operations 不顺带开启 Rename。
新增 `FileOperationRequest::Rename { from: VaultPath, expected: Revision, to: VaultPath, idempotency_key: String }`。
对应 `FileOperationKind::Rename { expected: Revision, to: VaultPath }`；沿用冻结记录的 `path=from`。
目标条件由 Rename 类型固定为 MustNotExist；没有覆盖选项、自动创建父目录、跨 Vault、批量或目录 Rename。
源/目标均须通过 whole-file 范围、保护区、真实根/父身份检查；同类别、不同规范路径、普通单链接文件、同设备。
大小写/NFC 别名、祖先前缀交叠、源目标同 inode、已有目标（包括零字节文件）均拒绝；不自动 canonicalize 接受别名。
HTTP 后续可用 `{kind:'rename',vault_uuid,from,to,expected_revision,idempotency_key}`；revision 仍为 64 lowerhex。
主人键域新增 `human.file.rename.v1`，不改 Delete/Restore 域；相同主人键换 from/to/expected/actor/scope 必须冲突。
现有 `FileOperationPolicy::allows` 只证明路径范围，不能证明目标扩展名/MIME/实际内容类别一致。
因此人侧开放前须补可信配对许可和目标内容核验；首片不把“都是 .md”或同扩展名当作范围/内容许可。
这不阻塞 Broker 的合法普通文件夹具与离线维护；若限定首个生产类别，须主线另明确类别及封闭核验规则。

## 2. 一次准备、两步效果、一次最终序号

一次 prepare 冻结两条 Observation（源 Present+expected、目标 Missing）和父目录证明，分配一个 operation_id/sequence。
两观察须在全部准备完成后再次核验；没有列出目标证据不能默认为 Missing。
同一次准备持久源 L0、目标 stage、目标 L0；目标 raw SHA/bytes 与源相同，路径绑定 revision 必须重新计算。
`stream/stage.rs::Input::History` 目前将目标计算 revision 与输入源 revision 比较，不能直接用于跨路径复制。
新增私有“验证源历史绑定并投影目标”的流式支路：固定源 FD/源 revision/raw，单次复制计算目标 revision，前后复验源证明。
保留原 History restore 比较原样；不得通过删除 revision 校验让两个路径共享旧 revision。
私有子步骤 ID 由父 operation_id 与固定 `target-install`/`source-detach` 域派生；没有独立外部请求/票据/水位。

| 阶段 | 可以执行/必须持久的事实 | 重开处理 |
|---|---|---|
| Preparing / Prepared | 两侧冻结、完整保全、预算；真实父 Intent ACK 后才允许安装 | 原票据核对或维护取消；没有安装事实 |
| TargetInstallIssued | 先持久，再 stage→目标 NOREPLACE；同步 stage 和目标父目录 | 保持未知；不能由目标同 hash 补成功 |
| TargetInstalled | 目标 raw/revision/inode、两父身份和目录同步的确切效果已持久 | 只提供审阅/保全维护，不自动删源 |
| SourceDetachIssued | 重新完整检查源与目标后持久，再源→私有 quarantine NOREPLACE | 保持源效果未知，目标已知效果保留 |
| SourceDetached | 隔离原件/raw/revision/inode、源 Missing、两父同步均已持久 | 可幂等完成原终态/投影，不再安装/摘名 |
| FinalizedAwaitAudit | 固定父 Outcome 和完整终态；只有实际 ACK 闭合才放行新写 | 只补原固定事件/投影 |

同目录 Rename 有专门的自影响问题：目标创建改变源父目录 Stamp；`Observation::check` 对旧 NodeProof 全等会自拒绝。
在 TargetInstalled 后重新流式观察源，要求原源文件 inode/稳定文件元数据、raw/expected 完全一致，所有父目录 inode 仍相同。
只重新固定该次源观察的父目录时间戳；同时核验目标已安装证明。不要放宽通用 file::check 或接受源 inode 替换。
两步在同一 commit/队列占用内同步执行；最后 Auth 后不再制作大 stage/L0，只执行必要的源/目标完整 CAS 核验和安装证明。
目标完成后的源冲突不能 Cancelled/Rejected；停在 TargetInstalled，等待保全维护，客户端返回 outcome_unknown。
维护完整封存双方当前可观察字节及原提案后，才固定 Indeterminate；不自动删目标、回填源或换键补跑后半步。
部分路径不可安全观测/保全时保留未决和引用，不将错误编码为 Missing，不以“现在两个文件都有”释放槽。

## 3. 双路径回执与不确定性闭集

沿用 `FileOperationReceipt`，Rename 的 effects 恰好两项，固定顺序 `[to, from]`；路径不可重复或交换。
Committed：目标 Confirmed(Missing→Present(target_revision/raw/bytes))，源 Confirmed(Present(source_revision/raw/bytes)→Missing)。
安装前固定 Rejected/Cancelled：两个 NotApplied。TargetInstallIssued 未知：目标 Unknown、源 NotApplied。
TargetInstalled 经维护：目标保留执行时 Confirmed、源 NotApplied；SourceDetachIssued 经维护：目标 Confirmed、源 Unknown。
Indeterminate 必须有核验过的 evidence_id；不能把已知目标效果抹成两个 Unknown，也不能从当前磁盘重建执行时效果。
SourceDetached 只有两项完整已持久效果才可 Committed；current 后续变化不改写历史终态或原键回执。
Committed 只推进一次 committed_through，原键重放不增加 sequence；Rejected/Cancelled/Indeterminate 不伪造成功水位。
半步期间沿用整体 read_blocked/未决门禁；完整视图发布继续由 owner 全量重新观察，不能输出半步“已移动”。

## 4. 版本与真实代码修改点

| 当前模块 | 所需最小修改 |
|---|---|
| `file_operation.rs`、`file_operation/{model,audit_model,status_model,frozen,request}.rs` | 新请求/kind/四阶段；两侧准备观察；新指纹版本，不改旧字段序列化 |
| `file_operation/{record,record/terminal}.rs` | Rename 专用进度/终态验证；两项闭集效果、阶段字段互斥、目标绑定 |
| `mutation/{record/state,replay}.rs` | 接受 State v6；replay 明确识别 terminal 5/6，不落旧 Vec 解码分支 |
| `file_operation/{prepare,prepared,commit,finish}.rs` | 一槽一次准备；取消仅首个 Issued 前；仅完整 SourceDetached 自动终结 |
| `file_operation/{prepare,store}.rs`、新增 `rename_enable.rs`、`file_operation.rs::phase` | 显式开启并验证 v6 标记；TargetInstalled 必须映射 Installing，不能落当前 wildcard Completed |
| 新 `file_operation/{rename_prepare,rename_target,rename_source}.rs` | 流式目标投影与两个有故障点的持久子步骤；旧 install 路径保持原语义 |
| `stream/stage.rs`、`blob/{file,store}.rs` | 有界跨路径 revision 计算/验证；复用 FD、NOREPLACE、fsync 和原始字节证明 |
| `file_operation/{maintenance_review,maintenance,evidence}.rs` | 双路径 review/六角色保全，各自正确逻辑路径绑定，新增阶段均能安全协调 |
| `maintenance/{model,record,target,audit,terminal}.rs` | Rename 双路径维护目标、schema 3/domain 3；旧 schema 1/2 原样 |
| `file_operation/{audit,tombstone,usage,capacity,limits}.rs` | 新审计版本、关联源墓碑、全引用/最坏容量与元数据布局 |
| `history/{file_operation,catalog,prune}.rs` | 两路径幂等投影/引用保护；不能只匹配 frozen.path |
| `local-server/src/file_operation_audit.rs` | 新 Rename 唯一映射/完整实际 AuditRecord 验证；不复制第二套映射 |
| `local-server/src/maintenance/{audit,presentation,workflow}.rs` | 展示源/目标当前可观察性、既有子效果和保全效果；TTY/双 ACK 不变 |

选定 State v6 + `rename_enabled` 持久标记；该 bool 默认为 false 且 false 不序列化，旧 v1—v5 State 字节完全不变。
显式 enable_file_rename 先核现有 v5 根/limits/绑定、唯一槽空闲、维护/恢复/原审计门禁闭合，再一次写入 State{version:6,rename_enabled:true}。
不新增目录、不重写原 binding.json、不自动补库；open_existing/prepare/原 enable_file_operations 都不能隐式升级，也不能从 6 降为 5。
严格闭集：v6 当且仅当 rename_enabled=true；v5/v6 当且仅当 file_operations_enabled=true；Rename active/终态要求 v6，新请求未显式开启即拒绝。
v6 内后续 Delete/Restore 仍写旧 Frozen/Request/Terminal 版本；普通写、维护保存必须保持 max(version,6)，已知旧终态仍可原键重放。
启用入口自身不改变 committed_through/next_sequence；失败不设置半个能力标记，旧可执行文件遇 v6 必须拒绝而非忽略未知字段。
选定 Rename Frozen/Request v2、Terminal format_version 6、Evidence v2；旧 Frozen/Request v1、Terminal 5、Evidence v1 原字节不变。
Active 可加仅 Rename 使用的有界可选进度；旧 None 省略，Rename 缺字段/旧操作携带字段均拒绝，不能默认补语义。
保持现有 operation_id 键空间（pair+keyhash）；只对 Rename 使用新指纹域，避免旧键被新版重新解释后再执行。
首片不需要第二存储根或新 crate。新增源码拆小文件，测试落 tests，均保持 ≤300 行。

## 5. 审计、维护和历史

选定 Rename Broker 事件 schema 2，Audit 外包装 schema 3；旧 Delete/Restore 保持 nested 1 + outer 2 及原 digest/ACK。
父 Intent/Outcome 仍各一条，action=`vault.mutation`；包含 from、to、expected、固定双效果，actor 永远是原真实主体。
新增 event_id 域只用于新版本；完整结构、阶段/效果、账本 Record/hash/offset 都要核验，不能仅 ACK 哈希字段。
维护 slot/event schema 3；旧 CLI 参数、TTY 身份/期限、原 Intent→维护 Intent→原 Outcome→维护 Outcome 真实 ACK 次序不变。
公共 MaintenanceTarget 增加仅 Rename 的 secondary 目标（路径、旧/目标 revision、current、proposed_available），主目标仍为源。
review_id 必须绑定整个双路径观察及已知子效果；ACK 不改审阅摘要，维护自行写入的升级不能使原票据漂移。
保全角色闭集：source_current、target_current、source_original、target_proposed、source_quarantine、target_stage。
每个角色显式选择源/目标 revision 域；沿用 Unobservable 原因，原提案/不可读 stage 仍保留引用而不伪装已封存。
只有完整 Committed 才发布源 tombstone，保留至少 7 天（以确定源摘名效果时间起计）；Rename 关联留在私有 tombstone v2。
最近删除公共条目可保持原形；不可因加入 renamed_to 而泄漏当前未批准目标路径。显式恢复仍只创建原路径，不反向删目标。
历史生成两个角色域 ID（父 operation_id+固定角色），同一 sequence，逐个幂等补齐；不是两个独立提交。
每个 catalog 条目绑定自身路径的 L0/revision 与父终态；可保持现有 catalog 3 结构，通过新角色 ID/终态版本封闭分流。
旧正文作者未知仍未知，移动/归档者单独记录；Indeterminate 保全不冒充整体 Committed 历史。

## 6. 容量、保留与失败可恢复性

当前 `capacity::reserve(body,deleting,executable)` 只有单路径公式；Rename 必须独立分支，不能把 body 乘二后复用。
设 B=本次实际源字节、M=当前 Broker `blob::OBJECT_LIMIT`（256 MiB，不是 Sync 的“200 MB”）。
可冻结保守预留：源/目标两份 L0+目标 stage 共 3B，未知隔离对象最多 M，六个证据角色最多 6M，再加全部元数据预算。
即私有 retained 上界用 `3B+7M+R_meta`；不扣除同 raw/reflink/去重节省，现有实际 retained 及旧未决预留都不能遗漏。
这是偏保守上界，允许因预算拒绝某些 256 MiB Rename；不提高现有文件限额或削减 7 天保留来让它通过。
分别检查 Vault/data 所在设备真实可用字节；同设备不重复承诺空闲，跨设备 L0 与 stage 则各自都满足保守最坏需求。
R_meta 从两条父链、进度、双效果、双路径维护/证据和 terminal/tombstone 的最坏 JSON 长度计算，全部 checked 运算。
必须在 Preparing/任何可见安装之前证明主日志/维护日志各 ≤64 KiB、Audit ≤1 MiB、其余固定目录总量可容纳；深路径预算不足早拒绝。
一源 tombstone 候选、一 stage 槽、两份历史对象/目录投影；保留现有 4096 墓碑与 stage 计数/字节上限。
usage/protected/prune 必须识别两个 revision、六个固定角色、两个子步骤及未 ACK/维护引用；未知不释放预留，不新增自动 GC。
磁盘被外部耗尽仍可能使保全失败；必须保持未决，不能把预算承诺误说成 OS 已预分配成功或允许新安装越过。

## 7. 首片必要回归与下一步顺序

1. 源缺失/expected 错、已有目标（含零字节）、父缺失、别名/前缀/链接/跨设备/跨范围类别；均不发生命名空间改变。
2. 正常同目录与跨已批准目录；raw 相同但两个 revision 不同；同目录自身父 Stamp 改变不自取消，真实源替换仍拒绝。
3. 目标出现竞争 NOREPLACE；TargetInstalled 后源改变、目标改变/父替换；保留两方，不盲删，不将已知效果改写为拒绝。
4. 每个 Issued 前后、fsync 前后、效果保存前后、终态/墓碑/两个 catalog/ACK 之间故障+重开；TargetInstalled 永不自动删源。
5. Issued 后当前内容碰巧符合预期、A→B→A、原键重放/不同动作同键；不得补成功/重复执行，换 to 的原键固定冲突。
6. 双效果顺序/重复路径/错误 target revision、缺进度/版本混搭/缺证明都拒绝；默认未开启、显式开启中断、反降级、旧 v1—v5 黄金字节/指纹/重放。
7. 双路径 current/历史/stage/quarantine 漂移、超限与不可观测；review ID 不因 ACK 漂移，证据完整前不清 pending。
8. 两项实际 AuditRecord ACK、阻断普通 Vec/recorded/stream 写和各 prune；无 ACK 不公布完成，Outcome ACK 后才恢复新写。
9. 容量边界/整数溢出/深父链/两份 L0+六角色/重复保全重开，近预算成功终态不因自己已批准的动作耗尽元数据。
10. 墓碑 7 天/原路径恢复、目标路径不泄漏、双 catalog 部分完成重开只补缺项，水位只推进一次。

建议先冻结版本/效果顺序/保留公式，再分三段实现：模型+流式 prepare；两阶段 commit+回执；双路径维护/审计/保留投影。
第三段完成并通过实际崩溃契约之前不启用 Rename；纯成功路径单测不能作为该能力可发布证据。
