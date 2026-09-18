# 整文件删除与最近删除恢复

2026-09-13，T-046。本文记录已接入代码的整文件操作；最终发布与真实身份验收仍未完成。
初始决策保留在 `file-operations-design.md`。

> **状态更正（2026-09-17 T-056 文档漂移审计）**：本页写于 Rename 落地之前，
> 原文称「未实现的 Rename 方案」并把人侧操作限定为 Delete 与 RestoreTombstone。
> 2026-09-14 起 Rename 已实现并接线：Broker 的显式开关
> `enable_file_rename` 见 `crates/vault-broker/src/file_operation/rename_enable.rs:12`，
> `FileOperationRequest::Rename` 见 `file_operation/model.rs:37`；
> HTTP 的 `{"kind":"rename", …}` 分支见
> `crates/local-server/src/file_operations/request.rs:14,20-26,48-67`；
> 能力查询已多返回 `rename_enabled` 字段（`file_operations.rs:53-55`）；
> 主人键域 `human.file.rename.v1` 见 `coordinator/file_operations.rs:76`。
> **下文凡以「只有 Delete / RestoreTombstone」「没有 Rename」为前提的句子只适用于写作当日**，
> 已在相应位置逐条标注。Rename 的现行权威是
> [`file-rename-contract.md`](file-rename-contract.md)（Broker 与审计/维护）与
> [`human-rename-contract.md`](human-rename-contract.md)（人侧配置、HTTP 与界面）。

## 产品行为

- 人侧只支持精确版本 Delete 与墓碑原路径 RestoreTombstone。恢复要求原路径不存在，零字节文件仍是存在，不覆盖任何同名文件。
  （**2026-09-17 更正**：Rename 已于其后加入，是第三种人侧操作，其目标条件同样固定为 MustNotExist；见
  [`human-rename-contract.md`](human-rename-contract.md)。本行其余关于 Delete / RestoreTombstone 的约束不变。）
- 删除前明确审阅并勾选当前已保存版本。任一次编辑/保存状态改变都会使原勾选失效，不能将 R1 的确认用到 R2。
- 编辑器、自动保存、页面历史恢复与文件操作共用未决门禁。发送中或结果未知时暂停后续写入。
- 最近删除需要真实会话与能力发现，分页每次请求32条；允许范围过滤后的空页继续下一页。恢复前重新明确确认。
- 关闭弹窗不丢原请求；迟到回调不重新读取列表或刷新已关闭界面。换主人后不暴露原主人的记录。
- 原请求只保存在本页内存，最多8个主人各一条。未决记录不能丢弃或自动换键；离页提示先导出请求信息。导出文件本身不授予权限，也不证明当前磁盘状态。
- 原键核对取得固定 Cancelled/Rejected 可结束未知状态；普通403、503或不完整200仍保留未知。

## 默认关闭与唯一写者

私有 `FileOperationPolicy` 配置见 `file-operations-config.md`。没有配置不启用存储，能力查询不初始化目录。
启动使用现有 CanonicalWorkbench 的唯一 Broker、真实根身份、实际 Vault UUID 与既有 AuditLedger；不另开写者。
HTTP 不接受 actor、scope、epoch、预算、任意恢复路径或覆盖开关。
可信 actor 来自当前 L2 会话；最终提交还要核验 CSRF、范围配置快照、实际根与捕获 epoch。
大正文保全和 stage 准备在最终授权前结束。

监视器只忽略固定 `Vault/.workbench/staging/<64位小写hex>.stage` 直属文件的已知创建、
字节写入、对应元数据及写关闭事件；所有路径须原样匹配，不能包含公开路径或别名。
Rescan、错误、模糊事件、重命名、删除、目录及身份文件变化继续失效。
路径过滤不构成暂存文件可信证明：提交仍由 Broker 核验 stage 完整字节、固定根和目标 CAS。

持久 Preparing/Prepared 后，失败只有在原票据取消且 Intent/Outcome 都取得真实审计 ACK 后，才可返回普通未执行错误。
Issued 或不能闭合的失败一律保持 outcome_unknown。丢失 Prepared 票据不从新请求重建；使用离线维护收口。
已知终态原键只核对真实历史回执和对应审计，不读取当前正文推断成功。

## HTTP

- `GET /api/v1/file-operations` → `{vault_uuid, enabled:true}`。无配置为403 disabled；不输出私有路径、预算或 scope 摘要。
- `POST /api/v1/file-operations`：封闭 JSON，32KiB、2秒、最多2048帧；kind 为 delete 或 restore_tombstone。
- Delete 包含 vault_uuid、path、expected_revision、idempotency_key；RestoreTombstone 将 expected_revision 换为 tombstone_id。
- 键为16至128位 ASCII 字母、数字、下划线或减号；revision/tombstone_id 为64位小写hex；路径最多4096 UTF-8字节。
- 真实主体、Vault、客户端键和不同动作域共同派生内部键。同客户端键的删除与恢复不会碰撞，同动作原键不能换条件。
- `GET /api/v1/tombstones?vault_uuid=...&limit=32[&cursor=...]`：HTTP limit范围1至64、cursor最多100字节；未知或重复参数拒绝。
- 分页只调用一次 Broker，再按当前完整文件批准过滤并复验配置。底层仍有最多4096条元数据的全读与排序成本；不输出被排除路径、actor或总数。

200 的 receipt 包括 operation_id、十进制字符串 sequence、committed outcome、一个 confirmed effect、tombstone_id 与 evidence_id。
Presence 明确区分 missing 与 present；present 带 revision、sha256、bytes。
公开 revision 均为hex字符串；Broker 内部旧 Revision 的32字节数组序列化保持兼容，不能混为同一 wire schema。
删除回执带新墓碑ID；恢复回执 tombstone_id 为null。回执证明原请求，不证明当前文件或远端同步仍相同。

409 的 file_operation_conflict/file_operation_cancelled 是固定原终态；idempotency_conflict 不允许重绑。
503 outcome_unknown 必须保留原请求；stale/budget_exceeded/audit_unavailable 只有在未形成新未决或已完整取消时使用。
401/403/429 沿用会话、范围和有界队列限制，不能自动提升权限。

## 磁盘与审计

主写者状态格式5与旧动作共享唯一槽、序号和水位。typed操作与旧正文操作互斥；旧请求指纹和旧维护事件保留原字节契约。
Delete 完整保全 L0 后以同设备不覆盖摘名移至私有隔离区；核对实际字节和原路径缺失后固定回执及墓碑。
RestoreTombstone 先验证原墓碑/L0并完整封存stage，再以 MustNotExist 安装原路径。
安装发出但未确认时，不能由“当前同内容”或“当前缺失”补成成功。

Broker typed事件内部schema1包在真实 AuditRecord payload2内；旧维护payload1不变，typed维护payload2。
维护保全明确记录 current、history 与实际stage；不可观察stage时保留未决，不能释放给普通prune。
正在安装、未知或Outcome尚未ACK时，公开当前读取和快照发布均阻断，包括空快照。
旧格式的LegacyUnknown同样拒绝公开读取；临时夹具直接核对磁盘原文不授予产品读取例外。
仅取消维护任务本身时，也先补原typed Intent，再确认维护Intent/Outcome；不产生原操作Outcome，原未决保持。

单对象硬上限256MiB；墓碑最多4096；实际预算受更窄私有配置限制，磁盘最低余量必须大于0。
未知槽按最坏3个大对象预留；已闭合操作释放未使用预留，实际保全文件继续计费。
没有墓碑GC，实际保留不少于7天并可能长期保留；容量不足拒绝新操作。没有 Rename 或任意路径恢复。
配置epoch没有跨管理员恢复旧私有配置的全局持久防回退地板；当前快照及原请求绑定不能替代该能力。

## 本轮证据与限制

- Broker typed专项34项通过；另1项是被外层实际启动的RSS子程序入口。含200MiB加17字节的删除/恢复与内存上限回归。
- typed维护合并后服务端库101项通过；人侧HTTP及监视器修复后库108项通过。
- 前端76文件630项通过；build、lint、format通过且没有构建或lint警告。
- 生产前端在本地4331合成接口完成删除、原路径恢复、刷新后墓碑发现与同名保护；390×844下无横向溢出，控制台无warn/error。临时服务及标签已关闭。
- 最终Auth及启动Audit等待期间的配置漂移已通过2项先失败后通过的真实回归；取消后的Outcome未取得审计ACK时保留未决。
- 监视器实际dispatch回归先失败（epoch由1误增至13），修复后两平台5项通过；包括推荐后端实际事件与私有根替换拒绝。HTTP整套41项通过，首次恢复必须200的严格断言保留，无重试助手。
- 本机首次workspace运行在旧LegacyUnknown读取断言处停止（1440通过、该项失败）；该断言已按统一门禁修正，保留原文字节验证。取消维护的真实账本顺序也先失败后修复。
- 修复后服务端22目标397项、writer30项、末尾36目标201项、文档16目标10项、架构31项均通过。以crate和目标去重的最新本机记录合计144目标1652项通过；这由首次运行及必要复跑组成，不将首次失败记作整套通过。两个ignore登记项是由外层实际运行的RSS子程序入口。
- 最终本机workspace严格Clippy、fmt、生产local-server/maintain构建、迁移到含空格独立目录的4组真实进程契约17项均通过。
- 同一957文件源码冻结版本v3，Linux aarch64单次完整workspace144目标1664项通过、失败0；两个RSS子程序入口由外层实际运行。严格Clippy/fmt、生产构建、迁移后4组进程契约17项均通过；编译警告0。此轮不包含尚在临时草稿的Rename。
- 这些不证明真实Cloudflare身份、生产服务、Obsidian双向往返、NAS恢复或两周运行验收。
