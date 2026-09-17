# T-046：CAS 删除、最近删除与安全重命名设计

私有批准入口见[配置边界](file-operations-config.md)；其本机契约已过，实际操作协议仍在实施。

2026-09-13。已完成设计复核与权威动作登记，产品实现仍待推进，未启用 HTTP、Bridge 或真实 Sync。
本稿基于现有 Broker、流式通路及权威 Vault；API 名称为建议，实施时冻结具体类型与版本兼容。

## 1. 权威要求与本片边界

权威根：`/Users/maxsong/Documents/Max's Secret Place/项目/Studiolo/`。

| 依据 | 必须保持的结论 |
|---|---|
| `03-记忆/ADR-002-运行工位与备份目标.md` | Server canonical 为活动权威；NAS 是恢复目标，Sync 不代替备份；备份追加身份不取得历史删除权。 |
| `03-记忆/ADR-005-Obsidian同步路径.md` | 唯一 Broker 写 canonical；双向 Sync、有界大附件、双方冲突字节保留、完整计划删除熔断。 |
| `03-记忆/ADR-010-内容平面写者归属.md` | 人写正文不能被 AI 整页改写；策略、运行投影、raw 和混合生成区不能借整页移动绕过。 |
| `01-框架/PRD/PRD-04-网页工作台.md` §7.2，行30 | 删除进入最近删除并至少保留7天；成功快照不能缩短窗口；重命名、移动、删除是条件能力。 |
| `01-框架/PRD/PRD-06-一致性与备份恢复.md` §10.4、§11 | CAS 冲突保留双方文本；人类/full 对象保全上一版；summary_only/none raw 不得先进入共享 L0。 |
| `01-框架/PRD/PRD-09-发布分期与验收.md` 行77、160 | 文件操作默认关闭，通过各自路径、历史、回收、容量验收后才启用。 |
| `01-框架/敏感动作枚举表.md` 四、T046 两节 | 现有人侧普通保存可补审计；恢复与事务维护失败关闭；新敏感动作先登记，不能扩大 AI 权限。 |

最小实现先覆盖单个已存在、已批准的普通文件；包括附件字节，不限 UTF-8。
不提供目录递归删除、目录树移动、覆盖目标、跨根/跨设备移动或“永久删除”按钮。
删除 raw 的到期生命周期不是本 API；不把 raw 套成人类正文再复制进共享历史。
针对已排除目录、策略、不可变证据和含生成区文件，源与目标均先拒绝，不能靠重命名改变类别。
浏览器按钮仍是条件能力；双向同步必须能安全表达删除，不能只因按钮关闭就遗留传输缺口。
Sync 的 rename 可以作为完整观察中的 create + delete 处理；智能识别 rename 并非 M2 的前置。

## 2. 当前实现不能直接复用的形状

仓库根：`/Users/maxsong/All-Project/studiolo/`。

| 当前文件 | 实施时须调整的具体假设 |
|---|---|
| `crates/vault-broker/src/recorded.rs` | MutationKind 只有 Replace/Restore；actor 与 kind 已进入原键指纹，新增类型不能伪装 Replace。 |
| `crates/vault-broker/src/mutation/record.rs` | Operation/Terminal/MutationReceipt 均依赖单路径和必有 target_revision/stage；不能用空文件 revision 表示缺失。 |
| `crates/vault-broker/src/content_audit.rs` | ContentAuditEvent 的 new_revision/sha256/bytes 必填；删除后缺失、重命名双效果须用类型表达。 |
| `crates/vault-broker/src/history/prune.rs` | 当前按至少50版、30天及恢复门禁保留；须加 tombstone、复合操作及未确认事件引用。 |
| `crates/vault-broker/src/recovery/prune.rs` | 当前识别单个活动 stage；新隔离对象不能放入可按孤儿 stage 回收的同一命名空间。 |
| `crates/vault-broker/src/maintenance/model.rs` | target_revision 必有，证据固定 proposed/current/previous；须识别缺失及两个路径。 |
| `crates/sync-bridge/src/{planner,evidence}.rs` | 三方规划只有元数据，不证明字节保全；Missing 必须来自完整观察，不能来自漏项或错误。 |

流式底座已整合到 `crates/vault-broker/src/blob/`；当前正收口历史证明与临时对象清理，完整回归仍待完成。
复用 `blob/file.rs` 的 no-follow/nonblock、唯一普通文件、父 FD/身份、ctime/mtime/大小重验，
`blob/digest.rs` 的64KiB块读取、raw SHA 与路径绑定 Revision，`blob/store.rs` 的封存与核验，
以及 `history/blob.rs`、流式 recovery 的有界保全。不另写一套 Vec 读整篇或路径式复制器。
该底座内部单对象上限256MiB；实际许可取显式 StreamLimits 与产品/部署上限的最小值。
其 stage 为64槽/512MiB，不能把7天最近删除永久占用这些槽而破坏正常吞吐。
建议代码落点：Broker 新 `file_operation/` 分 prepare/delete/rename/replay，`tombstone/` 分 catalog/restore/prune；
现有 mutation/history/content_audit/maintenance 只扩共享门禁与类型，local-server coordinator 负责真实身份、审计和新鲜度。

## 3. 建议公开 API 与类型

由同一个 FsVaultBroker 唯一 owner 执行；票据不 Clone、不反序列化、不接受调用者造证明。
以下方法均复核固定根、数据根、租约及当前活动槽；读状态也不能绕过损坏/绑定检查。

```rust
enum FileOperationRequest {
    Delete { path: VaultPath, expected: Revision, idempotency_key: String },
    Rename { from: VaultPath, expected: Revision, to: VaultPath,
             target: MustNotExist, idempotency_key: String },
}
enum FilePresence { Missing, Present { revision: Revision, sha256: String, bytes: u64 } }
enum FileEffect {
    Confirmed { path: VaultPath, before: FilePresence, after: FilePresence },
    NotApplied { path: VaultPath }, Unknown { path: VaultPath },
}
enum FileOperationOutcome { Committed, Rejected, Cancelled, Indeterminate }
struct FileOperationReceipt { operation_id: String, outcome: FileOperationOutcome,
    effects: Vec<FileEffect>, tombstone_id: Option<String>, evidence_id: Option<String> }

fn prepare_file_operation(&mut self, request: FileOperationRequest,
    metadata: FileOperationMetadata, limits: FileOperationLimits) -> Result<PreparedFileOperation>;
fn confirm_file_operation_intent(&mut self, ticket: &PreparedFileOperation,
    offset: u64, hash: &str) -> Result<()>;
fn commit_file_operation(&mut self, ticket: PreparedFileOperation) -> Result<FileOperationReceipt>;
fn cancel_file_operation(&mut self, ticket: PreparedFileOperation) -> Result<FileOperationReceipt>;
fn file_operation_replay(&self, request: &FileOperationRequest,
    metadata: &FileOperationMetadata) -> Result<Option<FileOperationReceipt>>;
fn file_operation_status(&self, operation_id: &str) -> Result<FileOperationStatus>;
fn pending_file_operation_audit(&self, limit: usize) -> Result<Vec<FileOperationAuditCandidate>>;
fn confirm_file_operation_audit(&mut self, candidate: FileOperationAuditCandidate,
    offset: u64, hash: &str) -> Result<()>;
fn list_tombstones(&self, cursor: Option<TombstoneCursor>, limit: usize) -> Result<TombstonePage>;
```

这是签名草案，Result 统一以 BrokerError 为错误类型；MustNotExist 是封闭常量而非布尔覆盖开关。
effects 最多两项且顺序固定；未知效果与未执行不同，不造 Missing；NotApplied 也不声明当前文件状态。
Metadata 由装配提供实际 Human/Obsidian actor；浏览器 JSON 不接受 actor、许可或保存来源。
Broker 不自行认证会话；可信 owner 在流式 prepare 之后、同步 commit 之前重新核验实际身份/撤权和范围。
Broker 只核对冻结请求及其能力/根；不能凭 metadata 的 issuer/subject 或 scope_hash 假装完成授权。
原键查重作用域跨 Replace/Restore/Delete/Rename；同键不同类型、路径、条件、主体均冲突。
指纹包含格式域、逻辑 Vault/pair、两路径及各自条件、主体、原键摘要、范围代次与保留语义。
重试核对原完整请求；已知拒绝保持原409，已知成功重放原回执，未知只返回原未知状态。
当前路径不存在不能充当“删除已成功”；重新出现相同内容也不能使旧删除键再次执行。
状态、回执和审计只有逻辑路径/摘要/版本/状态，不暴露正文、秘密或私有磁盘路径。

最近删除恢复复用既有 L2 `kind:restore` 动作，新增封闭 TombstoneRef 输入：
必须选择同一批准路径、有效 tombstone 和稳定原字节，以 MustNotExist 创建新的 revision。
已重建同名文件则409，禁止覆盖；恢复是新幂等请求，不改旧删除终态，不提前消费墓碑。
最小版本仅恢复到原路径；另存路径、批量恢复属于后续能力。列表不是正文授权凭证。

## 4. 删除：先保全，再摘名，再确认

1. 全部逻辑路径/范围/预算先验；源必须存在且 expected 精确匹配。缺失也是固定 CAS 拒绝。
   路径大小写碰撞、隐藏/保护区、根/父替换、链接、多硬链接、FIFO/socket、设备均拒绝。
2. 从固定源 FD 流式读取，验证实际旧字节、raw SHA、路径绑定 revision 及前后元数据。
   旧字节先进入受信 L0，fsync 文件及目录，回读核验；失败不改变 canonical 命名空间。
3. 为本次操作预留最近删除槽与字节预算；记录不可变 tombstone candidate 与对象引用。
   固定 Intent 和 Prepared 日志都持久后返回票据；任何新写/恢复/维护不能越过活动槽。
4. 真正 AuditLedger Intent 经完整事件及回执核对后 ACK；commit 再检查范围/根/源 CAS。
   范围与身份由可信 owner 最后准入复验，Broker 接着同步执行，不在两者之间排队等待后台任务。
5. 先持久 `DetachIssued`，再将源目录项以 NOREPLACE 原子移入同设备私有隔离区。
   隔离区建议 `.workbench/deleted-objects/`，独立于 `.workbench/staging/`，禁止通用读写。
   名称只由 operation_id 派生，预占重名一律拒绝；两个目录均持有已复核的 FD。
6. fsync 源父目录和隔离目录；核对被摘名对象与已封存旧字节及原 inode 身份相符。
   rename 自身可改变 ctime，不能把“修改后的时间等于旧时间”当成功条件；重新稳定读取核验。
   核对源确已 Missing、根/父仍是本次批准对象后，持久 `Detached` 精确效果记录。
7. 发布 Committed tombstone、固定终态/序号和 Outcome。Outcome 真 ACK 后才宣布完成与开放后续写。
   私有隔离原件是附加保全，不是唯一恢复副本；L0 已验证字节才是稳定恢复输入。

不直接用 std::remove_file/canonicalize，也不在检查后执行无日志 unlink。
同根摘名可保住最后竞争替入的目录项，避免错误对象被不可逆删除；它不提供内核级 revision CAS。
CAS 保证依赖既有“唯一受信 canonical writer”边界；外部同权限进程可破坏该边界。
若最末检查后遭非法替换，必须保留隔离对象和已封存原文、报告 Indeterminate/篡改，不能声称未改动。
不得自动把未知对象放回原路径，因为这会覆盖后来的用户文件；不能跟随隔离区中的异常链接。
挂载跨设备不作复制后 unlink 降级；启动审批保证隔离目录与 canonical 同设备，失败关闭。

## 5. 重命名：关联复制后删除，不宣称双路径原子

只支持同根、两个不同规范化路径、目标不存在、源为已批准普通文件；父目录先存在。
大小写改名、文件与目录前缀重叠、同 inode、多链接、跨范围类别均先拒绝。
目标扩展名/MIME 与实际字节必须符合目标许可，不能借移动跳过上传/下载的内容分类和安全响应规则。
源 revision 含逻辑路径，因此目标 revision 必须重新计算；相同 raw SHA 不表示两者 revision 相同。

1. prepare 一次审批源/目标、冻结两侧条件，流式保全源 L0，同时制作目标 stage 和目标 L0。
   私有子步骤 ID 从父 operation_id 与固定步骤名派生，不暴露可被外部调用的独立写权限。
2. 父 Intent ACK 后，持久 `TargetInstallIssued`；以 NOREPLACE 安装目标，核验目标并同步父目录。
3. 持久 `TargetInstalled` 的确切目标效果。此时源仍存在；整个复合操作仍占唯一 writer 槽。
4. 重新完整核对源 expected、目标刚安装版本/身份、两父目录和范围；已知冲突则停止，保留两份。
   不能先取出 source BlobSource、随后无条件删源；不能因已复制目标而允许覆盖目标的新版本。
5. 持久 `SourceDetachIssued`，按上一节摘名并保全源；它生成关联到父操作的源 tombstone。
6. 只有两步都有执行时的持久效果、目录同步和固定终态，才得到整体 Committed 与同一最终水位。
   Outcome 记录两个路径的前后状态；发布新鲜度时需要同一逻辑变化集，不把半步包装成已移动。

目标安装后的已知源冲突可固定为 Indeterminate，附“已知目标创建、源未删除”的结构化效果。
固定此终态前必须流式封存可验证的双方当前字节和原提案；不能只凭“两个路径还在”释放证据引用。
这里 Indeterminate 表示整体未按意图完成，不抹去已知子效果；不是普通 CAS Rejected/NoEffect。
不自动回滚目标、不自动删除冲突源，不用新键补跑剩余步骤；用户审阅后另发新的 CAS 请求。
重开若停在已确认 TargetInstalled，只报告待协调，不自行继续破坏性源删除。
出现 Install/DetachIssued 无后继确切效果时仍未知；就算两路径现在恰好符合目标，也不能补成功。
若已有完整持久终态但 ACK/投影缺失，只补固定事件/投影；其后文件变化不改写历史回执。

## 6. 日志、墓碑与保留回收

增加 versioned FileOperation 分支，允许有界一/两路径的冻结请求、阶段、效果和保全引用。
新写启用标记同时进入既有主日志/绑定验证；活动标记缺槽、未知版本、交叉绑定一律失败关闭。
不能仅新增旁路 JSON、让旧 write_status 仍显示 Ready；所有普通写、Restore、prune、维护均检查它。
首次启用必须由明确装配/初始化完成，open_existing 不自动补表或缺失目录。
旧 V1/V2 请求保留原协议重放；不能改旧指纹域或按新 enum 默认值重新解释旧终态。

墓碑固定记录：版本、tombstone_id、operation_id、Vault/pair、原逻辑路径、旧 revision、raw SHA、
字节数、稳定 L0 引用、scope/source policy、原 actor、原请求指纹、确定提交序号、服务器时间、
最早清理时间、关联 rename、固定审计状态。候选/已提交/未知/已退休明确区分。
不保存客户端原幂等明文键；不以 path 当唯一主键，同名重建再删除产生独立墓碑。
零字节文件仍是 Present，不与 Missing 合并。缺失只作为有证据的状态，不编造空正文对象。

硬约束是提交以后至少7×24小时可恢复，Prepared 等待时间不抵扣；未知时间、未知终态均保留。
系统时钟回退不得缩短保留，跳前不能直接取得提前回收资格。最小保守方案：
自动墓碑清理默认关闭；显式清理同时要求期限满足及本 owner 连续单调观察已提交对象至少7天。
重开丢失单调证据则重新计满7天，允许保留更久；不引入客户端时间或新外部授时依赖。
若以后改为跨重启可信计龄，应单独证明时间来源与回退行为；不能直接复用调用者传入 now。
成功快照、手机已同步、恢复过一次、Bridge 计划退休都不能缩短7天；L0的50版/30天规则独立保留。

清理根集合至少包括：全部活动操作、两子步骤、未 ACK Intent/Outcome、维护槽、未知证据、
未到期 tombstone、未决恢复意图、未完成 checkpoint/恢复保留引用；缺失或损坏引用则整体拒绝。
先持久固定回收候选及授权审计 ACK，再逐项重验身份/摘要与引用，最后 unlink/fsync、提交 retired。
候选最多64，重复执行同一回收计划；中途失败不报全完成，保留仍未清理对象及原事件。
墓碑 metadata/body、隔离原件、L0 blob 的引用分别处理，不因只清一个目录而丢失恢复关系。
最近删除条目可到期退役，但原请求拒重信息仍保留；任何键回收只能永久 Retired 拒绝，不能忘后再执行。

资源预算必须包括累计 tombstone 数/字节、隔离原件、L0新增字节及复合 stage，不能只计算请求正文。
建议显式 FileOperationLimits：现有单文件上限、硬最多4096活墓碑、配置批准的总保留字节上限、
磁盘预留；无明确配置则不启用。增量按最坏真实分配保守计费，不依赖 reflink/稀疏文件/去重节省。
单复合操作最多两路径，一槽；L0/隔离区各自的逻辑上限及物理空间均检查，不能借新目录逃逸总量。
容量不足拒绝新的破坏性操作，禁止提前清7天副本；备份上传成功不释放该预算承诺。
扫描/列表分页只限元数据，先核上限后分配；恢复/证据读取仍逐块校验，不以大 JSON/base64 返正文。

## 7. 固定审计与维护衔接

建议沿 `vault.mutation` 增 Delete/Rename 子类，另用 typed FileOperationAuditEvent 表达缺失/双效果。
Intent/Outcome 共享稳定事件身份，绑定实际主体、请求指纹及有序路径条件；不可重试时重取时间/ID。
ACK 必须核对 AuditRecord 完整性、同一 Ledger、事件内容/phase、offset/hash，不能只接收任意整数。
必须在权威表明确这两个破坏性子类失败关闭；不能暗改既有普通人侧 replace 的“执行并补记”。
同步摄入/导出沿 Bridge 固定 action Intent/Outcome 并单独核对实际文件 owner 回执，不套人侧例外。
父事件和子效果 ID 有固定关系；Bridge 不得凭相同 hash 创建第二份 delete/rename 成功事件。
文件日志的执行终态先持久、再追加 Outcome/ACK；审计故障返回 outcome_unknown 并阻新写。
commit/replay 返回的底层 receipt 只是持久执行事实；FileOperationStatus 必须另含 audit_pending。
HTTP 与 Bridge adapter 在真实 Outcome ACK 前不能把它投影成“完成”；审计候选至多两阶段、按原顺序补记。
已知 CAS 拒绝/未执行取消也写固定终态；不能因事务日志暂时不可读而报告普通失败并让新键重试。

维护保持现有实际 UID/GID、非提权、前台 TTY、独占租约与双 ACK；不伪造原 Human/Obsidian actor。
`cancel-prepared` 仅适用于尚无任何可见效果的复合 Prepared；目标一旦开始安装就不可按它取消。
`preserve-installing` 增加源/目标各自的 Present/Missing/Unobservable，封存可核验字节及私有候选。
最多两个路径的旧/当前/拟安装对象去重引用；分块保存并检查总证据预算，不能退回4MiB Vec。
证据完整才固定 Indeterminate；遇 unsafe/current 无法读取不跟链接、不假装 Missing，保持未决或明确不可观察证据。
`cancel-maintenance` 仍只取消未执行维护槽；`maintenance_pending` 与 audit-flush 不重新读取 unsafe 原目标。
维护不能把 DetachIssued+Missing 或两路径相同摘要补成功；安装后成功只能来自先前持久的执行证据。
原 actor、文件操作结果和维护自己的 Applied/Cancelled/NotApplied 三层不能合并。

## 8. Sync、checkpoint 与发布登记

SyncTransport 只管理自己的专用镜像和自有 headless 子进程；确认其退出后才进行镜像 CAS 安装。
镜像 stage/最近删除对象位于镜像之外的已批准私有目录；同设备要求在启动审批时核验。
不能把 FsVaultBroker 冒充镜像 owner；后续窄 MirrorFileOwner 使用同级原键/阶段/保全协议。
暂停本地 headless 不冻结远端；再次下载必须完整重观察，两侧有变化就保双方，不推进旧共同基线。
首次单侧缺失、扫描错误、根消失、不完整下载均不能产生 Delete；先建立可信完整共同基线。
计划含删除大于5或超过旧完整存活路径10%，整计划在任何副作用前持久熔断；rename 不能规避删除计数。
源删除与目标创建的已知子效果仍不能让 Bridge 整体提前 all_committed/settle。
如果外部 rename 以独立 create/delete 观察，保持三方各自 CAS 与冲突字节，不自动按哈希合并动作。

当前 BridgeState 草稿 `sqlite/{model,receipts}.rs` 已接受 Delete 的 new_revision/hash/bytes 全为 None，
但 EffectOutcome 只有 Committed/NoEffect。成功 Delete 可以精确适配，不需为了“缺失”改其字段。
Broker 的 Indeterminate 不能映射为 CasRejected/CancelledBeforeInstall；当前必须维持 OutcomeUnknown。
真正传输接线前若要在人工保全后继续，需冻结第三种有原终态/证据的 Indeterminate 回执与状态机：
它只表示该旧动作已被收口为未完成，all_committed=false，保留旧基线，重新审阅；不把旧动作补成功。
必须登记该收口及审计/退休条件并补测试；不能简单让旧 unknown 行变 Known 或开“清除未决”开关。
当前两动作纯规划不承载一个复合 Rename 回执；本片不用它合并两个不同原键，避免虚构同一父事务。

checkpoint 清单新增操作日志、墓碑目录/元数据、私有摘名对象、固定审计 outbox、GC计划与证据引用。
和实际 Broker owner 同一冻结屏障，不能分别复制后声称一致；引用对象缺失则 checkpoint 不完整。
恢复新根保留逻辑 Vault/pair/原键/终态，按既有物理 rebinding 协议重绑，不据现存文件重建成功。
旧 checkpoint 与较新镜像基线不匹配时暂停破坏性同步重新协调；不删除未决以便启动。

以下边界已在权威《T-046-文件操作与最近删除动作登记》中登记并由敏感动作矩阵链接；实现仍需逐项验收：

| 登记位置 | 最小增量 |
|---|---|
| `01-框架/敏感动作枚举表.md` 四 `POST /api/v1/mutations` 与 T046 | 人 L2 Delete/Rename 封闭参数、两侧范围、默认关闭、审计失败关闭；普通 Replace 保持原规则。 |
| 同表 T046 单页恢复 | 墓碑 restore = 新请求+原路径 MustNotExist，L2/失败关闭；不是整库恢复或强制覆盖。 |
| 同表 AI 动作矩阵 | 明确本片不新增 AI 删除/重命名人正文权限；未知 action 默认拒绝，不能继承“内容写入”。 |
| 同表 Bridge 内部事件节 | 明确绑定 link/Vault/scope 的实际 delete/rename 子类与镜像 owner；维护后的 Indeterminate 收口、旧基线与退休条件；既有 bridge.action/熔断/退休不另造身份。 |
| 同表维护节 | preserve-installing 可含双路径/缺失；Prepared 取消边界；不解除未知/审计门禁。 |
| 同表新增保留清理内部动作 | 建议 `vault.retention_authorized`，仅 owner 有界固定清理计划、真实 Intent ACK，独立于 NAS 管理清理身份。 |
| `03-记忆/ADR-005-Obsidian同步路径.md`、`ADR-010-内容平面写者归属.md` 与 `01-框架/写路径架构图.md` | 沿已登记 SyncTransport 分阶段镜像所有权补精确入口；不改“唯一 Broker canonical”或赋权消费者。 |
| `01-框架/PRD` 对应条目、T046计划/TASKS/日志 | 记录条件能力是否真正启用、最近删除7天验证、原子性限制及真实 Sync 验收尚缺项。 |
| 仓库 `docs/m2/{human-content-contract,writer-maintenance,bridge-state,checkpoint-recovery-contract}.md` 与新文件操作契约 | 固定新 DTO、版本、事件 payload、阶段、水位、限额、保留及恢复清单；AuditLedger action 为字符串，不需伪造不存在的 Rust 事件枚举。 |

以上权威路径均已在 Vault 定位；事件仍由仓库对应契约固定 payload，不自行新增同义端点。

## 9. 可执行验收切片

1. 先 Broker 删除+墓碑+create-only restore；已有流式底座通过后再开新类型，路径和格式负测优先。
2. 后复合 Rename；失败阶段、双路径 CAS、维护与水位齐全前不接浏览器按钮。
3. 可信 owner/HTTP 接线：真实 L2+Origin/CSRF、范围重验、默认关闭、AI拒绝、固定审计失败关闭。
4. Sync adapter 后续接实际回执；完整计划熔断、镜像生命周期、真实设备往返/两周试跑仍单独验收。

必须用确定性故障点覆盖：旧正文保全前后、Intent持久/ACK前后、Issued持久后、系统摘名后、
两个父 fsync 之间、效果日志后、终态前后、Outcome append/ACK之间、GC每个unlink前后。
每点真实重开验证：有确切旧终态才重放；否则原键未知、字节仍保全、禁止后续写或错误维护取消。
Rename 再逐点覆盖目标安装成功后源变更、目标再被编辑、源摘名后崩溃，不自动补源删除或回滚目标。
固定根/父替换、大小写碰撞、符号/硬链接、FIFO、源同长修改、同内容ABA、多级父变化均有真实FS负测。
对最后系统调用窗口注入异常替换，验证不宣称 NoEffect、不丢封存原文、不误清隔离对象；不靠随机竞态。
正文至少包括零字节、非UTF8、中文路径、真实42MB级和200MiB级非稀疏文件，检查每字节 SHA 与峰值内存。
限额精确值/加一、目标预占、磁盘满/fsync失败、累计墓碑耗尽必须拒绝且不提前删旧恢复点。
7天前/等号/之后、时钟跳前回退、重开失去单调证据、备份成功、同名重建、恢复一次后的保留都要测。
原键跨类型/主体/两路径冲突，Audit故障、撤权、未决维护、原内容/维护双 ACK 均须端到端覆盖。
checkpoint 恢复到新物理根后保留墓碑/未知/拒重；L0、stage、墓碑三类 prune 均不能误删这些引用。
macOS/Linux 分别验证 NOREPLACE、同设备摘名、父目录 fsync 和真实重开；不支持平台直接拒绝启用。

可延后：自动 rename 识别、链接批量改写、目录树操作、覆盖型移动、跨设备搬运、快速去重/reflink、
后台自动 GC、任意恢复目标、批量 UI。不能延后：已启用删除的旧字节保全、7天最近删除、未知状态、
源/目标 CAS、固定原键与审计、删除熔断、权限排除、实际流式大附件及恢复验证。
