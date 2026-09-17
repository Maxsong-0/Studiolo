# 完整本地观察内核（T-046）

2026-09-14，接续既有 canonical 完整观察与 2026-09-13 Mirror 观察候选。
本页规定已实现的本地观察单元；macOS 与 Linux aarch64 的受影响接口验证均通过，
不代表生产同步已可用。实现与证据见文末。

## 范围与固定 API

将已有元数据/完整字节读取拆为两阶段，供 canonical 与显式只读根复用。
不引入新依赖、进程、同步许可或对外 HTTP 入口，不开第二个 canonical Broker。

- `FsVaultBroker::begin_sync_observation(selection, required)` 返回封闭 `CanonicalSyncSurvey`。
- `finish_sync_observation(survey, additional_required)` 消费原 survey，返回既有 `CanonicalSyncObservation`。
- 原 `capture_sync_observation` 保持兼容，作为两阶段顺序包装；旧 ready/read_blocked、根/实例/配对/提交水位门禁保持。
- `ReadOnlyVault::begin_scoped_observation` / `finish_scoped_observation` 返回封闭 `LocalReadSurvey` / `LocalReadObservation`；另有同 reader 的最终重验。
- 两种 survey 的 `paths()` 只读公开本次发现/精确规则/已知 required 路径，用于汇合完整集合。
- 本地只读 Present 只含原始 SHA256、长度和完整 UTF-8 事实，不产生 canonical revision、不标为格式已批准或 Mirror 已静止。

survey 保存不可替换的 selection、初始 required、全部访问名字/目录/文件证明以及 owner 实例。
字段无公开构造、Deserialize、通用 builder 或授权 trait；新参数只能增加同一 selection 内的 required，不能变规则。
两侧先取得 survey，汇合 U = baseline ∪ canonical 路径 ∪ 另一侧路径 ∪ 精确文件，再各自完成 U。
另一侧新路径不能在外部 DTO 中直接补 Missing；须用原完整名字和固定父链重新证明明确 ENOENT。
无法取得静止或完整共同观察时，不推进 Bridge baseline、不补造旧动作成功。

## 读取与预算

沿用现有 canonical 预算与大小写/Unicode 保守索引：全部直属原始名字先核碰撞再过滤，硬排除零正文且不下探。
保留 64 目录规则、128 精确文件、320 额外排除、4096 目录/最终路径、1024 字节路径/64 深度；
全体枚举触及第 65536 项拒绝，名字累计 4MiB，索引 66048 项/8MiB。
required 合并后再完整校验，包括与已访问隐藏/排除名字的冲突；规则变更、被排除路径、单次额外列表内重复及超限整体拒绝。
额外列表与已发现/原 required/精确规则重合允许，取集合并，调用方可以直接提交完整 U。

元数据阶段零正文，首个正文打开前确认全批预算；单对象不超过规则与 256MiB 较小值，所选正文累计不超过 4GiB。
字节阶段固定 64KiB 缓冲、完整摘要和跨块 UTF-8 事实，不保留正文，不额外读探测字节。
旧 blob transfer 的超限探测/安装行为保持；可复用 facts 的可选 namespace，不伪造 canonical 身份。
`SyncScope::approve_union` 仍另行执行 baseline+canonical+mirror 三组 Present 合计 4GiB 及联合路径预算。

每一步及最终复核全部目录 stamp、对象/父链、Missing 证明；拒链接、非普通/多链接/跨设备、截断/增长/换 inode。
只读 reader 的 proof 绑定同一打开实例，重开同一路径也不能接管旧 proof；不暴露 FD。
`open_private` 的原有属主、0700 目录、0600 文件要求在共享内核前后继续执行，不能借新 API 绕过。
固定根被移动时沿用既有 reader 的句柄语义；生产 Mirror owner 另外负责核对批准的宿主路径和静止窗口。

## 实现与验收

文件域：`vault-broker/src/sync_observation/` 内核和模型；`readonly/sync.rs`、必要最小导出/reader实例字段；
`blob/observed.rs` / `digest.rs` 的事实分支；对应 scoped/two-phase 实际临时文件系统契约。
新增/改动普通源码不超过300行；已有超限 lib 仅增加最小导出，不整文件改写。
根独占 repo 整合与串行工具链，草稿不得改 repo 或运行 Cargo/rustfmt。

必须覆盖：双方各有新路径后完整 Present/Missing、缺祖先、空目录、额外 required 被排除/Unicode碰撞、
元数据后新增/移除/替换、读中漂移、proof跨 reader/新实例/水位拒绝、private模式、硬排除不读正文、
真实流式大文件及有界RSS；既有 canonical 正反例和旧一次式 API 全部保留。
本地事实通过不等于真实 OwnedRun、MirrorCAS、Bridge执行器、附件格式验证、远端往返或两周试跑完成。

## 本轮实际结果

已整合15个文件；共享 `sync_observation/kernel.rs`、两种封闭 survey/observation、
`readonly/sync.rs` 的私有模式和实例复核，以及旧一次式 API 包装均按上述边界实现。
没有新增依赖或修改旧 blob transfer 的读取/安装语义；继承超限的 Broker lib 仅增加导出，
其余15文件范围内普通源码最长230行。主线核对8份前像、7个新路径和11份未改输入后整合。

1013个构建输入冻结点在macOS、Linux aarch64各通过7组102项：
原23项与新增16项观察契约、25项旧只读/字节接口、7项文档封闭性、31项架构检查。
额外登记的1项ignore是由父测试实际调用的RSS子程序，不是跳过验收。
实际写入200MiB+17字节并由两入口分别完整读取；内存峰值分别6619136和8028160字节，均低于128MiB。
两平台fmt、workspace/all-targets严格Clippy与生产构建通过，期间源码未变化。
这是观察增量的受影响范围验收；此前1006输入的重命名版本另完成双平台全workspace，不能混加为一次全库数字。
日志、前后输入摘要和整合文件摘要保存在仓库忽略的 `target/t046-20260914`。
