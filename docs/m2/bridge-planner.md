# 双向 Bridge 三方规划契约

T-046，2026-09-13。源码已整合到 `studiolo-sync-bridge`，35 项纯规划契约已通过。
仅元数据规划，尚未持久化、未接入真实同步。

## 权威与冻结

已读 Vault ADR-005 第 19–36/51–57 行、ADR-010 第 34–44 行、
PRD04 第 29–30 行，以及仓库 docs/m2/sync-transport-design.md 第 45–84 行。
主线已冻结：双目录、精确范围、双侧显式 Missing、完整批次、无未决成功推断，
删除数 >5 或占原完整共同基线存活路径 >10% 时整计划阻断；等号可过。
这里不决定 headless 受控出站写者、持久化、熔断解除或实际附件字节上限。

## 输入与输出

- 唯一 workspace 依赖是 `studiolo_core::VaultPath`；其余使用 Rust 标准库。
- `ApprovedScope.paths` 是可信 scope adapter 对本次完整联合路径逐项核准的输入，
  不是用户白名单配置，更不是规划器能自行添加的授权。
- scope_hash 必须绑定规则真实内容及版本，不能只绑定名字。装配还须负责路径碰撞、
  硬排除、生成区、source/full-scope 与整页复制许可，本核不推断 Markdown scope。
- Baseline、Canonical、Mirror、Cursor 必须与同一 Binding 精确相同。
- 两侧 Inventory 必须完整并逐项列出 scope 的所有路径，缺失显式用 Missing。
  Baseline 包含所有已管理路径，且每项双方同内容或均 Missing；新路径可无旧基线。
  校验只能检测输入间矛盾，不能证明扫描器遗漏了真实文件，也不能认证调用方。
- baseline.version 同时匹配 expected_baseline_version 和 cursor.baseline_version。
  新 plan_sequence 必须为上次 +1；两侧观察序号严格前进，基线观察不能来自未来。
- `Decision::Ready` 只包含有源版本和目标条件的提案、冲突与共同观察建议。
  `FuseBlocked` 没有动作字段；本批普通复制也不先执行，后续持久层必须先保存熔断。
- 输出按 VaultPath 排序，每路径最多一个动作/冲突/共同观察。现存共同基线的存活
  路径数是比例分母；新建文件、已经共同 Missing 的路径不能扩大分母。
- 默认每批最多 4096 路径/输出，单路径最多 1024 UTF-8 字节。
  这里没有正文，接入层须在分配集合前施加同等解码预算。
  `Limits::conservative(max_file_bytes)` 必须显式传入实际字节上限，没有 200MB 换算。
  copy_bytes 采用有检查的加法；传输队列总量、并发、stage 与磁盘预算仍待接入。

## 基线、回执和重试

同侧未变以该侧 revision 和内容证据精确比较；双方同内容以 sha256 + bytes 比较。
不同侧 revision 可以不同，不得拿镜像 revision 替代 Broker expected revision。
相同 revision 却正文证据不同、相同 hash 却大小不同均拒绝。

初次单侧存在只生成 create-only；首次双方不同生成冲突。已有基线的单侧变化产生
条件复制；删除对未变是条件删除提案，删除对修改保留双方证据并生成冲突。
纯核只有元数据，执行者仍须在任何破坏性操作前真实保全字节与可恢复删除记录，
保留至少 7 天；不能把元数据冲突项冒充已保存冲突副本。

plan key 是 link_id + scope_epoch + 单调 plan_sequence；action key 另含原路径、
目标侧及动作种类。A→B→A 使用新序号；相同内容 hash 不是幂等键。
存在旧 pending 时不允许新规划，即使双方碰巧相同。`retry(original, frozen)`
仅接受全等冻结输入并返回原 keys。它不是磁盘核验，也不是已安装/同步成功证明。
调用者必须先持久保存计划、游标和原 pending，并在执行时重验源与目标条件。
未知结果的原键核对与共同基线提交由后续状态机负责，本核没有成功回执入口。

## 验证范围

测试覆盖首次导入/分歧、双方独立 revision、双改与删除对修改、显式缺失、完整性、
全部身份绑定、基线/观察序号、全批两方向熔断、边界等号、新文件扩分母负测、
分页削减基线负测、整数溢出、预算、旧 pending、全等重试和 A→B→A。
上述 35 项纯规划契约已通过 macOS 与 Linux aarch64，并纳入完整工作区回归，
两端格式化与严格 Clippy 通过。纯契约不能替代真实 Sync 验收。
