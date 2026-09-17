# T-046 人侧 Rename 实施契约

日期：2026-09-13；2026-09-14更新验收。仓库 HEAD 4f6f348，工作树含主线未提交增量。私有策略、HTTP 与界面均已整合；专项测试及合成浏览器通过。1006输入冻结点的macOS完整workspace1719项和Linux aarch64完整workspace1731项通过；fmt、严格Clippy、生产构建及两平台独立目录17项进程契约通过。未在真实 Vault 启用，本文不是整体 M2 验收记录。

## 1. 结论与必要边界

人侧最小闭环可先批准一种明确的内容类 `markdown_text_v1`，上限 4 MiB：由私有配置给源、目标各自整文件许可，再以同一 Broker 的完整有界读取验证实际文本字节，绑定 source revision/raw/bytes，并依赖 Rename 已冻结的无转码、同 raw/bytes 的目标投影证明。这里不需要另开 canonical 根、使用通用扫描器或新增附件解析依赖。

这不是“两个名字都是 .md 即有权移动”：私有类别批准、两路径整文件授权、完整字节验收、源 expected 与目标投影一致、最终真实会话/配置/根复核缺一不可。现有 `file_operations` 开关和 `HumanContentScope` 均不能自动获得 Rename 类别批准。

首片不批准 PDF/图片/音视频/ZIP 等附件 Rename。当前仓库没有这些格式的完整有界验证器；头部魔数、后缀、浏览器 MIME 或 `SyncAttachmentType` 枚举都不能补足。以后若批准附件，要先按每个具体容器实现完整验证和资源预算，再新增显式类别版本，不能借本次文本类型扩展。

Markdown 没有可证明唯一身份的文件魔数，普通文本也是合法 Markdown 表示。`markdown_text_v1` 必须明确为“主人登记为 Markdown 的合规 UTF-8 文本”，不是内容嗅探器猜出的文件格式。完整字节验收证明它属于这个明确定义的文本集合；不声称鉴别作者意图、没有别的解释方式，或已验证某种附件。目标按照该文本协议处理，不能因字节里出现 HTML/PDF 字样转给附件或活动内容处理器。

## 2. 实施前可复用代码与待补接口

下表保留本片实施前的接口拆分依据；表中列出的 Rename 适配现已按本契约整合，实际验证状态见整体收尾台账。下列路径均相对 `/Users/maxsong/All-Project/studiolo`。

| 现有落点 | 已有证明与应复用行为 | 不能冒称已有的能力 |
|---|---|---|
| `crates/host-config/src/file_operations.rs::FileOperationPolicy::{allows,revalidate}` | 私有 Snapshot 前后重验，完整配置实际字节 hash，Vault/epoch/预算，整文件范围 | 仅 bool 路径许可；不证明文件内容、类别或双路径配对 |
| `crates/host-config/src/human_content.rs::{allows,allows_whole_file}` | 主人明确登记，保护区/保留名/任何生成区登记优先；删哨兵不能扩权 | 目录中每个后缀不是自动 MIME 许可；混合文档部分编辑权不是整页 Rename 权 |
| `crates/vault-broker/src/capture.rs::FsVaultBroker::read_file` | 同一个 writer 的固定 canonical 根、真实 namespace/revision、有界完整字节；前后 ready/read_blocked 水位检查 | 返回 Vec；不应把 256 MiB 统统读进文本内存 |
| `crates/vault-broker/src/sync_observation{.rs,/model.rs}` | 完整批准本地范围、SHA/bytes/revision、全 UTF-8 事实与 sealed proof | `utf8_valid` 不含控制字符策略、Markdown 语义或附件格式；Sync 批量选择不授人侧权 |
| `crates/vault-broker/src/blob/{observed,digest}.rs` | 分块完整摘要、稳定 NodeProof、父链/单链接/同设备；UTF-8 跨块尾部处理 | 没有 MIME 分类；不能把一个前缀或自报枚举当成完整格式证明 |
| `crates/local-server/src/coordinator/file_operations.rs` | 唯一 owner lock，原主体键域和完整请求指纹、replay 在 freshness 之前、同 Broker 执行 | 目前只核 `input.path`，Kind 只有 Delete/Restore；Rename 必须同时核 from/to |
| `crates/local-server/src/coordinator/file_operations/{access,execution}.rs` | prepare 在最后 Auth 前完成，实际 Intent ACK，最终 principal.verify 后再次校 roots/policy/Broker，失败取消和真实 ACK 收口 | 尚无双路径类别许可与实际字节绑定 |
| `crates/local-server/src/auth_http/{input,gate,content}.rs` | 原 Access 身份＋session/CSRF，POST 同源门禁、JSON 32 KiB/2 秒/2048 frame、持有原主体供最后 verify | 不能从客户端接受 scope、类别批准、真实 MIME、预算或预验证标记 |
| `web/src/{fileOperationModel,fileOperationValidation,useFileOperations,EditorController}.ts` | 有界原请求内存记录、会话 scope、失败留原键、草稿/保存门禁 | 现模型/校验按单路径 Delete/Restore；不能把新增两个 effects 当一项旧回执接受 |

`ReadOnlyVault::read_file` 使用 replica namespace；不能用它的 revision 替代 canonical CAS。`CanonicalReadVault` 虽能只读真实 canonical，也不是本 owner 的 unresolved/队列门禁；人侧不应绕过已有 `FsVaultBroker::read_file` 去新开一个根。

## 3. 最小私有策略/API

建议在既有 `file_operations` 对象内新增缺省为 None 的严格对象 `rename`，旧合法配置原样有效、仍只批准 Delete/Restore；显式 null、未知字段、数组形式均拒绝。配置形状候选：

```json
{
  "rename": {
    "version": 1,
    "whole_folders": [
      {"path":"项目/写作","class":"markdown_text_v1","max_bytes":4194304}
    ],
    "whole_files": []
  }
}
```

这是 `file_operations` 的局部示例，不是一份可直接部署的完整主人配置。它继承同一已核 Vault、scope_epoch 和实际完整配置 hash，`rename.version=1` 不替代外层版本。新增/改变批准须按现有范围变更流程提高 scope_epoch；不更改旧 Delete/Restore 的事件版本、键域或默认行为。

规则最多 64 个目录、128 个精确文件，新增段实际编码总量不超过 16 KiB；无规则不得当成全 Vault。同一保守 Unicode/NFC/大小写前缀索引验证规则、保护区、生成区和候选两路径；重复、别名、父子规则重叠均拒绝，不靠“最具体规则优先”解决歧义。可复用 host-config `sync_scope/paths.rs` 的内部实现，但只导出最窄 crate 私有校验函数，不给 Sync 权限或复用一个可公开伪造的观察来取得批准。

建议封闭 API：

- `FileOperationPolicy::rename_policy() -> Option<&FileRenamePolicy>`：只有可信配置装配能取得，无公开构造/Deserialize；不从 HTTP 提供。
- `FileRenamePolicy::approve_pair(vault, from, to) -> Result<ApprovedRenamePair, ConfigError>`：前后同一 Snapshot 重验；两个路径分别先通过原 `allows_whole_file`，再各自命中恰好一条同 class 的新规则；取两规则与外层预算最小值。目标尚不存在也必须有规则，Missing 不能免授权。
- `ApprovedRenamePair` 私有字段绑定 vault/from/to、类别版本、固定 MIME、确切后缀集合、实际 scope_hash/epoch、max_bytes。仅只读 getter/revalidate，不具备写权限；不能经 builder 增类、增上限或更换另一侧路径。

首类仅精确小写 `.md`，文件语义 MIME 固定 `text/markdown; charset=utf-8`。后缀在取得类别规则之后才检查，作为类别的表示约束而非授权来源；不接受 `.MD` 大小写别名或客户端覆盖 MIME。判定末尾后缀须为精确 `.md`；此前名称中的点号不另作类型推断，例如 `v1.2.md` 仍按同一已批准文本协议核验。若将来允许第二种合法后缀，必须显式改类别版本/配置，不能以“同扩展名”默认归类。

完整配置 hash 已覆盖新增批准、类别版本和预算。当前旧键的 metadata 指纹包含这个 hash；改变配置后旧请求可能按现有边界返回 idempotency_conflict，而非透明回放。这是已有绑定约束，不能为“兼容”偷偷改用新的 scope/actor 补认旧请求或发新键；固定审计/离线维护仍处理原未决。

## 4. 实际内容证明与目标一致性

建议在 local-server owner 私有模块新增有界文本 admission，不新增通用扫描能力。

1. 取得并保留同一 owner lock；真正核验 principal、roots 和 both-path `ApprovedRenamePair` 后，才读源正文。任一侧未批准时零正文 I/O、零 prepare/日志/审计副作用。
2. `broker.read_file(from, min(pair.max_bytes, 4 MiB))` 完整读取；核 vault/path/revision 必须等于输入 expected。正文已完成全部读取才可判定，不能只检前 512 字节。
3. `markdown_text_v1` 的字节语言固定为完整有效 UTF-8，拒 NUL、除 TAB/CR/LF 外 C0 控制符以及 DEL/C1 控制符；不转码、不替换错误字节、不规范化换行或正文 Unicode。空文本与纯文字明文明确允许；允许 UTF-8 BOM（U+FEFF）并保留原字节；不将它删除或转码，也不把它作为权限或类型证明。该类别是文本表示约束，不承诺去除 Markdown 的 HTML/链接，继续由既有安全渲染处理。
4. 根据这一份完整 bytes 计算原始 SHA/bytes、source canonical Revision 及 `Revision::compute(vault,to,bytes)`；后者必须不同于 source revision。封闭的 `RenameContentAdmission` 仅保存这些小元数据、类别版本和 pair binding，不输出正文到 HTTP、日志或审计；大 Vec 在 prepare 前释放。
5. 构造 Rename 请求；Broker `prepare_file_operation` 重新固定源 Present(expected)＋目标 MustNotExist，完整源 L0/目标 stage/目标 L0 均在准备阶段完成。ticket.preview.observed_before 的 revision/raw/bytes 必须与 admission 完全一致，固定 path/kind/to/expected 也必须相同；Rejected 只能走原固定拒绝回执，不能当成已验证提案执行。
6. 目标不存在，不能“读取目标来验 MIME”。证明来自 Broker 冻结契约的跨路径投影不变量：目标 stage/L0 的 raw 与字节数等于已经分类的源，目标 revision 用目标路径计算，prepare/commit 均核对封闭 ContentProof。因此目标实际拟写 bytes 与 admission 是同一内容、同类别、同 MIME 表示；不是根据目标名猜测。此传递证明须由 Broker 实际 stage 篡改/源替换/跨路径 revision 契约支持，不增加一个自报 `verified:true` 字段。
7. 沿现有真实原 Intent ACK → 最终 file_gate → commit → Outcome ACK。最终 principal.verify 返回后仍在同一 control lock 内，重验原 roots/配置 Snapshot/pair binding、实际 Broker roots；任何漂移走既有 cancel＋ACK 路径，不能把未闭合取消误报为安全拒绝。此时不再做正文分类或 stage/L0 大制作；Broker 自身必须完成的最终双路径 CAS 不得削弱。

首片最多一份 4 MiB 源内容及小元数据；不因为底层支持 256 MiB 而扩大 HTTP 文本读预算。以后要支持 256 MiB 文本可把同一完整文本谓词移入已有流式 digest/facts 并返回 sealed admission，但本片无需先增加那套公开能力。附件不能复用此文字谓词；PNG/PDF/ZIP 等独立完整校验缺失前保持拒绝。

## 5. 启动、HTTP 与原键重放

启动：在 `Core::enable_approved_file_operations` 的同一 trusted roots/真实 Audit 查询/查询后重验链中，只有取得非空且合法 rename 私有批准、服务已含 schema3＋维护适配，才显式调用同一个 Broker 的 `enable_file_rename()`；前后 roots/policy/Broker identity 均验证。单独 enable_file_operations 不调用 Rename；没有配置的 reader、MCP、旧 CLI builder 不给它权限。持久 v6 标记本身也不能替代本次主人配置批准；撤掉 rename 批准后接口默认禁用，不降级主日志版本或停止原审计补记。

启动恢复补充（2026-09-13）：`file_rename_enabled()` 只读并校验当前主日志和文件操作存储绑定，不初始化、确认或执行动作。只有该真实标志为 false 时才调用原严格 enable 方法；已启用且有待补审计时跳过重复授予，让原 outbox 继续闭合。true 不能解释成当前可写、身份已批准或审计已完成；capability 同时要求当前私有批准与真实持久标志。该补充保持原 enable 方法对未决状态的严格拒绝，须以真实 SQLite Outcome 故障后重启验证。

实际 SQLite 重启测试进一步暴露同根因：重复调用 `enable_file_operations()` 也会拒绝原待补动作。新增只读 `file_operation_limits()` 校验主日志/存储并返回实际已启用预算；装配仅在未启用时调用严格 enable，预算不相等仍拒绝，不修改原 enable 门禁。查询不写日志、不确认事件、不产生能力。Rename 和旧配置的 Delete/Restore 均已实际验证 Outcome 故障后重开、沿原键补记且不重复安装。

POST 沿既有 `/api/v1/file-operations`，新增严格 `{kind:"rename",vault_uuid,from,to,expected_revision,idempotency_key}`。from/to 各限 4096 字节，均规范路径且非同路径/别名/前缀交叠，JSON 总量仍 32 KiB。禁止 body、MIME、class、scope、预算、overwrite 或另一个目标条件。目标父目录须已存在，不创建父目录。

原主体＋Vault＋原客户端键使用新域 `human.file.rename.v1` 派生 Broker key；不要把 from/to/expected 加入这个外层键派生，否则换条件会变成另一键绕过冲突。它们已进入 Broker 的完整固定指纹；同键换目标/源/expected/actor/scope 必须拒绝。旧 Delete/Restore 域与版本原样。

重复请求先按原会话和两路径当前许可核对，再查 `file_operation_replay`；不得先读取源正文或要求 fresh generation，因为已成功源可能 Missing。回放只补固定事件，核完整实际 ACK，并在返回前再次核双路径许可。不能因为某条路径现在失去授权而泄漏包含目标路径的旧 receipt。未终结 Prepared/Issued 不经此 HTTP 重派，不给新 expected 或新键“修复”。

新请求仅在无旧固定请求后才进入完整内容核验/prepare。后续 source 消失但原键已有 Committed 时仍回放同一固定双效果，不宣称当前两个路径仍相同。原永久 Indeterminate 继续 outcome_unknown，不通过同 hash/现在两个文件都有补成功。

GET capability 必须单独返回 `rename_enabled`（缺省 false），不能沿用一个 enabled=true 推断所有动作。前端 discover/validation 必须分别维护 Delete/Restore 和 Rename 能力，配置关闭时不初始化存储。最近删除列表仍只发布源 tombstone 的已批准字段；不增 renamed_to 暴露未批准目标。

## 6. UI 最小闭环

复用 `EditorController.canRestoreHistory/beginFileOperation`、同一个 FileOperationStore/ScopeBoundary，不引入第二个 in-flight 状态。Rename 审阅同时显示源已保存版本、目标完整逻辑路径、“目标须不存在、不会覆盖或创建父目录”。草稿、IME、自动保存、历史恢复或任一文件操作未决时不允许开始；任何 editor snapshot、已保存 revision 或目标输入变化都使已勾选确认失效。

提交前一次性冻结 from/to/expected/原键；双击只复用这份原请求。目标输入不能在 in-flight 中改变后继续沿用旧确认。scope 切换/关闭组件只取消视图更新，不删除内存原请求、不自动生成新键，也不把网络 abort 当作取消执行。

Committed 只接受完整 `[to,from]` 双 Confirmed、path/revision/raw/bytes/原 operation_id 对应的严格回执，再请求实际新快照，按目标路径重新打开；不直接把旧源 editor base.path 改成目标而沿用旧 revision。没有经过整批新鲜视图不能在树上乐观删除源/出现目标。

目标以新 EditorController 完成独立读取后，原历史回执仍在同 Vault 的源或目标视图中显示，可导出或明确收起；只有源视图提供“重新读取目标”。收起只清内存操作记录，不重载正文、不丢目标草稿。实际浏览器发现并修复了“跳转后回执不可见而操作持续禁用”，回归覆盖目标可编辑与目标已有草稿两种状态。

outcome_unknown 文案：“重命名结果尚未确认；目标可能已创建，源是否已摘除未确认。保留两个路径和原请求标识，先核对原请求。”只有明确固定效果返回时才能写“目标已确认安装”；当前泛化 Problem 没有这些字段，首片只显示保守未知，不猜具体半步。保留原键核对/导出，不提供撤销半步、删目标、覆盖目标或换键重试。

后续若要在页面显示已知目标效果，可增加只投影经过当前双路径权限验证的固定回执事实；不能调用全维护审阅把私有证据或别路径带到 HTTP。本片无需先增新的通用操作状态端点。最近删除“恢复源”只恢复源，不隐式删已建立的 Rename 目标。

## 7. 可独立验收的必要清单

- 私有配置缺失/只有旧开关：Rename capability false、POST disabled、没有新初始化；合法旧配置/旧 payload 不变。非法类型、null、规则重叠/Unicode别名、保护区/生成区规则全拒；删哨兵不能取得许可。
- 源获许可而目标无许可、类别不同、任一侧预算更小、目标不在 human whole-file 范围：正文读取前失败。相同 .md 但无新类别批准仍失败。
- 完整字节负测：无效 UTF-8 在中间或末尾、截断多字节、NUL/控制字节、真实非文本附件改名 .md、超 4 MiB、边界空文本/中文/BOM/CRLF；不替换字节让本来不合格的对象通过。ASCII 外观的任意文本按明确文本语言处理，不冒称附件识别。
- 一份实际完整读取与 prepare/目标投影绑定：源在分类后变更、stage/目标 L0 篡改、目标临时出现、同父目录自影响、最终源/目标父根变化；不得输出成功或把目标 overwrite。两份 actual raw/bytes 相等、canonical revisions 不同。
- 最终 SessionClock verify 暂停后撤权/配置漂移，以及启动 Audit query 暂停后撤批准：零新能力/零可见安装；Prepared 后失败必须保留真实取消＋ACK 状态，未闭合继续 unknown。
- 同一个 owner 与查询/写入许可：第二个并发请求排队或 busy，不可创建第二 Broker；关闭/队列超限不遗失已持久原请求。故障跨四个 Rename 阶段与两 ACK 后重开沿 Broker 原协议。
- 原键 Committed 回放在源 Missing 时成功、同键改 to/expected 409、失去目标权限不返回双路径旧 receipt、原 unknown 只核对不执行、审计失败不可假终局；旧 Delete/Restore 契约继续完整运行。
- UI 真实编辑器/DraftStore：勾选后保存 R2/改目标使确认失效；双击、关闭后迟到响应、切 scope、自动保存与历史恢复互斥；严格双效果回执；半步未知只保留原请求，不乐观改树。

## 8. 实施落点与范围分界

最窄新增：host-config `file_operations/rename.rs`（封闭规则与 pair）和 local-server `coordinator/file_operations/rename.rs`（4 MiB 实际文本 admission）；对现有 request/access/execution/capability 增加 Rename 分支；web 模型/validation/store 与单个 Rename 审阅组件适配。复用原唯一 audit mapper，不加第二套执行/审计回调。所有新增源码拆成不超过 300 行的文件。

主线已选定本文的文本字节语言、原字节保留BOM、私有 rename 配置、显式类别及 4 MiB 上限、capability/双路径响应校验；后续按这些闭集实施。该决定不授予任何实际Vault目录的生产批准。对应 owner 实现、真实身份/HTTP/两平台/浏览器验收完成前，人侧 Rename 保持关闭。

不在本片：附件格式验证器、任意 MIME/后缀转换、目录 Rename、跨 Vault/覆盖目标/自动建父目录、文件内链接重写、自动回滚半步、远端 Sync/冲突规划或 NAS 接线、M6 运行体。Broker Rename 的流式 256 MiB 能力与离线维护可独立验收，不等于上述人侧扩权已发生。
