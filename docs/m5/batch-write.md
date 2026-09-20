# M5 批量写回与候选 staging

T-088 提供 `BatchWriter`、`ArtifactStager` 与 `NumberingScanner` 三个独立能力。
它们由既有 `FsVaultBroker` 和 `MemoryVaultBroker` 实现；既有 `VaultBroker` trait 和
`ArtifactCommit` 调用方式继续可用。TaskEngine 应显式声明所需 trait，不能自行打开 Vault 路径。

## 批量计划和预览

`BatchRequest { batch_id, changes }` 固定一批 1–32 个 Markdown 路径。每项为
`Create { content }` 或 `Patch { expected, patches }`；每个 `TextPatch { before, after }`
必须唯一匹配非空旧片段，按顺序执行。没有批量完整覆盖既有正文的入口。范围授权、AI/人主体
判断仍属于 TaskEngine/Auth；Broker 不凭传入字符串认定主体。

输入补丁文本以及全部旧/新正文分别受 4 MiB 总预算限制。拒绝受保护路径、重复路径、大小写
撞名、冲突父目录拼写和祖先/后代目标。新文必须为无 NUL 的 UTF-8 Markdown；若含开头 YAML
frontmatter，则必须闭合、可解析并为映射。Markdown 正文允许通常的自由文本。

`preview_batch` 先核全批 CAS，再返回按路径排序的完整 before/after diff。`fingerprint`
绑定协议域、Vault UUID、完整原请求和全部 diff。`commit_batch` 再次生成预览、完整比较并
重验 CAS；篡改任一预览内容或复用批次 ID 提交其他请求均被拒绝。预览只用于审阅，正文不能
写入 Control DB。确认/审计准入由调用方提供，本 trait 本身不假装是用户确认机制。

## 同一持久事务

磁盘实现继续持有唯一 canonical writer 的 OS 租约，在既有 `writer-state.json` 的 v7
格式增加互斥 `batch` 槽；它与单项 mutation、流式 mutation、文件操作及维护操作不能并行。
使用同一已绑定 canonical/data 根、同一 staging 目录、同一 L0 history 和提交水位。

1. 全批检查 CAS，保存全部旧/新正文到私有 L0，再同步所有待安装 stage。
2. 再核全批 CAS 和 stage/history 身份；在 canonical 控制目录持久保存不含正文的读屏障。
3. 持久保存 Prepared 与 Installing journal，逐项通过既有安全安装内核安装。
4. 复核全部目标的 revision、摘要、大小和 stage inode，持久保存 Completed 及全批回执。
5. 只推进一次已提交水位，清除唯一 batch 槽，最后清除读屏障。

在任何已占槽阶段，Fs 的公开读取、捕获、快照创建、编号扫描、history 读取和 canonical
写操作全部失败关闭；独立 `CanonicalReadVault` 也在读取前后检查持久读屏障。全部状态只在
完成后发布。直接绕过 Broker 的原始文件系统访问不具备事务隔离；普通 `ReadOnlyVault`
用于本地副本观察，也不能代替 canonical 一致读。

重开仍由原写者 journal 驱动。每项只有“旧 CAS + 完整 stage”或“目标 revision + 原 stage
inode/摘要”的证据成立才允许前滚；先预检整批，再继续剩余安装。外部冲突保留未决与屏障，
不猜测成功、不暴露已完成的一部分；记录或保全内容损坏可能直接拒绝打开。

## 整批回滚

`rollback_batch(batch_id)` 支持成功批次，也支持当前尚未公开的同批未决操作。成功批次必须
先对全部目标核对原安装证据；任一文件被后续写入，全批零修改拒绝。旧正文从路径绑定的 L0
revision 读取并核验，原本不存在的文件恢复为不存在。回滚也占一个 journal 槽，使用相同
持久屏障；中断重开继续整批回滚。已回滚批次重放回滚返回原结果，重放原提交返回已取消。

未决正向批次切换回滚时，只允许每项仍为原文或本批安装版本；后来的人工内容不可覆盖。
L0 history 整理会保留 batch 回执所引用的原/新 revision；批次正文不进入 journal。
当前批次保全对象由批次回滚接口访问，未把它们伪装成带人侧作者/时间的公开 history 条目。

Memory adapter 在全批 CAS/命名空间检查成功以后一次替换对象表，保留同样的 preview、patch、
幂等和 rollback 冲突契约；它不能证明磁盘崩溃或路径安全语义。

## 候选正文和控制指针

`ArtifactStager::stage_artifact_commit(&ArtifactCommit)` 将完整候选保存到 Broker 私有 data
根，返回 `StagedArtifactCommit { id, sha256 }`。`id` 绑定 Vault UUID 和原始候选摘要；
`load_staged_artifact_commit` 必须重验两者与规范编码，不接受内容篡改、跨 Vault 指针或缺失对象。
指针不含正文、文件系统环境路径或候选文件名。

候选最多 64 项、正文 4 MiB、编码记录 8 MiB；私有库最多 4096 条、累计 64 MiB。重复相同
候选复用内容寻址记录。TaskEngine 的 reservation/outbox 只存该指针，恢复时通过 Broker
重新装载正文，再调用既有 manifest-last `commit_attempt_artifacts`。staging 本身不写 canonical。

## 编号

`NumberingScanner::scan_number_max(scope, prefix)` 使用 Broker 的有界可信扫描，递归匹配
scope 中普通文件名的 `prefix + 十进制数字 + '-' 或 '.'`，仅返回最大编号。缺失范围为 0；
完整扫描出现不安全诊断或超预算时拒绝，不能把部分结果当成上界。数据库还须在事务中取
`max(文件编号, 已预留编号) + 1`，扫描结果本身不授予唯一号码。

## 验收范围

私有单元测试覆盖全批 CAS 零修改拒绝、预览篡改、精确 patch、重复/祖先路径、YAML、
Prepared/Installing/首项安装/Completed/回执五个故障阶段的重开、整批回滚的相同中断窗口、
回滚拒绝后续人工修改、未决切换回滚、独立 canonical reader 屏障、候选指针正文隔离与
篡改拒绝，以及编号扫描。

集成者执行：`cargo test -p studiolo-vault-broker`、`cargo fmt --all -- --check` 和
`cargo clippy -p studiolo-vault-broker --all-targets -- -D warnings`。Linux 路径语义须在 githubci
原生回路验证；故障注入只是进程级阶段证据，不等同于断电或存储设备持久性认证。

## 结构化记忆去重（T-090 Broker 部分）

`MemoryWriter::preview_memory(&MemoryEntryRequest)` 只读扫描所给 `03-记忆` 范围的既有
Markdown。类型和摘要经过 trim、NFC、Unicode 完整 casefold，再 NFC 生成去重键；特殊
casefold 表基于 Unicode 16，涵盖 ß/ss、希腊末尾 sigma、ligature 等 lowercase 不等价情况。
既有人工条目无需特定文件名或 dedupe_key，只要 frontmatter 的 type/summary 对应就返回
`Existing { path, revision, sha256, requires_patch: true }`；候选 body 不覆盖它。同范围多个
已有条目同时命中时拒绝，要求先归并，不能任意挑选一个。

不存在时返回 `Create { request: BatchRequest, preview: BatchPreview }`，目标文件名为
`memory-<去重SHA256>.md`，frontmatter 含 type、summary、date 和 dedupe_key。date 严格为
有效公历 YYYY-MM-DD；正文含 frontmatter 推荐至多 200 行，`exceeds_target_lines()` 提示
超出推荐值，300 行为硬上限。单请求是一种类型、一个摘要和一份正文，不合并多个事项。
Broker 仅认可路径中精确的 `03-记忆` 分量，具体项目范围授权仍由调用方复核。

通用 `BatchWriter` 的预览和最终提交也会检查 `03-记忆` 下的全部候选：type/summary/date、
300 行上限，并按同一个 `03-记忆` 根目录对本批与当前库去重（忽略本批被替换路径）。因此
另取文件名调用普通 batch.create 或 patch 也不能制造同义条目；较早预览之后新出现的人工
同义条目会在最终 batch commit 再扫描时拒绝。根编排仍须在同一 Broker 锁内重验用户预览、
claim/fence/审计并提交。已有正文需要另行构造精确 patch；记忆预览不会自动提交替换。

以上只提供 Broker 能力，授权、用户可见确认和 Control DB 指针持久化属于根编排。记忆正文
与完整 Create 预览仍不得写入 Control DB。
