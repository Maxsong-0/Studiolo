# 隔离恢复副本撤权契约（T-094）

2026-09-20：既有Auth237项通过；装配时钟修复后，107恢复16项及CLI34项、108 root未来时钟夹具1项通过。
108真实副本已确认原v2绑定、epoch2→3、清除1条旧L3及10枚恢复码、保留时钟/限流/历史证明，幂等审计通过。
快照唯一会话已过期，实际新增撤销会话数为0；未做真实Cookie重放或恢复后真人登录/L3。
新能力未部署生产，不宣称完整灾备闭环；详见[实际回执](../../deploy/human/offline/ACCEPTANCE.md)。

## 入口与授权边界

```rust
pub fn invalidate_restored_credentials(
    &self,
    restore_id: &str,
) -> Result<RestoreInvalidation, SessionError>
```

该方法属于`SessionManager`；`RestoreInvalidation`从`studiolo_auth`根导出。
`restore_id`只接受1～55位ASCII字母数字或连字符，非法值返回`SessionError::Configuration`。
API只交给可信离线装配：调用方冻结全部副本写者、阻止服务监听，保持原Auth和匹配的Audit一起恢复。
API本身不检查宿主路径、UID或网络隔离，不证明备份来源，不提供Access/L2/L3或HTTP撤权入口。
[管理CLI](auth-admin.md)在其前施加固定目录、root、独立netns及秘密stdin门禁。

CLI通过`SessionManager::open_existing_for_restore(root, config, audit, clock, random)`装配：
签名参数与普通`open_existing`一致，只允许ExistingOnly/MigrationNone/v4并保留存储/配置检查，
不执行`SessionManager::run`，不采样注入时钟、不推进时间地板或到期状态。普通打开入口的运行检查保持原样。

外层包装器先验证完整解密与全部清单，再copy-to-new；不可对生产库调用本API。
此入口保留已有因子，不能用来绕过丢失因子、换主人、登记新种子或生成恢复码。
相关既有契约：[TOTP](auth-totp.md)、[恢复码与因子替换](auth-recovery.md)、[HTTP](auth-http.md)。

## 撤权的原子范围

第一次使用一个恢复ID时，要求已有合法v4、已登记因子，且TOTP pending、Recovery pending和retention计划均为空。
登记前置由恢复API直接读取既有证明检查；CLI只用`Totp::attach`确认种子绑定，不调用会推进运行时钟的`TotpEnrollment::state()`。
不自动创建/迁移表，不清pending或重置猜码预算来满足前置条件。
校验epoch和事件序列溢出、有界outbox剩余容量后，捕获Auth快照与当前保留证明。

原登记/因子、恢复及L3证明在Auth写锁外提交给原Audit重确认；返回的完整回执必须逐字匹配已存回执。
错账本、空替换账本、伪造或错位回执不能授权撤权。重新取得Auth事务后再捕获并比较全表状态摘要，
任何并发变化均拒绝沿用旧快照。该摘要只在内部使用，不对外输出。

以下变化与两条恢复outbox事件在一个Auth事务中提交：

1. 全部active/pending会话变为revoked；原epoch加1，旧会话凭据不能因副本上线而复活。
2. 删除全部`totp_grants`，使旧L3证明失效。
3. 删除当前`recovery_codes`全部行（包含已消费行），清空current批次并将该批次used_mask置0。
4. 写入同一恢复ID的intent/outcome事件，记录原epoch推进与实际计数，执行最终schema校验。

当前因子/登记证明、因子引用和绑定、历史审计、时钟地板、猜码预算、TOTP消费高水位、
恢复候选高水位与恢复历史不会因此重置；清空当前恢复码集合不会重新启用任何旧码。
后续新会话、重新L3与新恢复码须走既有受控流程，本API不签发任何凭据。

## 审计交付与结果

Auth事务提交后，原两条事件送入指定Audit账本。两库不是跨库原子事务：审计失败不会回滚已经完成的撤权。
事件action为`session.revoked`、scope为`restore`，effective为`already_revoked`，payload版本为1。
事件ID为`<store_id>:restore:<restore_id>`，同一ID使用intent/outcome两阶段。
封闭payload八字段为`scope`、`target`、`revoked`、`identity_version`、`effective`、`cleared_l3`、
`invalidated_recovery_code_count`、`version`；审计内部计数字段与公开结果字段名称不同。

| 公开字段 | 类型 | 含义 |
|---|---|---|
| `epoch` | `i64` | 此恢复ID首次提交后的身份版本 |
| `revoked_sessions` | `usize` | 首次实际转为revoked的active/pending会话数 |
| `cleared_l3` | `usize` | 首次删除的L3证明数 |
| `invalidated_recovery_codes` | `usize` | 首次废止的当前恢复码行数，含已消费行 |
| `audit_confirmed` | `bool` | 本次重核依赖证明及两条原恢复事件是否获精确确认 |

CLI只输出这些五个非秘密字段。`audit_confirmed=false`仍输出JSON但以非零退出，副本必须保持离线。
提交后的I/O、路径复核或stdout错误也可能返回错误而撤权已生效；先核对原ID和权威状态，不能把错误当作回滚。
`audit_confirmed=true`只是指定事件确认，不是新登录、L3或服务上线许可。

## 同ID永久锚点与容量

同一个`restore_id`永久复用原epoch、计数和原事件，不再次推进epoch或重复撤权。
即使之前未取得回执，也必须先重核仍保留因子的原证明属于同一Audit，再补交原恢复事件；
不能换空Audit来“修复”失败。已有回执必须精确匹配，不能接受另一合法偏移。
该调用用于操作者核对同一恢复决定；不允许包装器自动生成新ID掩盖失败或无条件循环重试。

两条恢复outbox事件是永久幂等锚点，retention不删除它们；旧会话终态可按既有证明规则回收。
每个新ID占用两条安全事件；当前共享安全outbox上限8192，容量不足返回`SessionError::Capacity`。
不能通过删除历史锚点、清库或复用别人的ID腾空间；8192不是可完成8192次恢复的承诺。

## 程序兼容与实际验收边界

持久库版本仍为v4，但增加封闭`scope=restore`事件语义及epoch/retention验证规则。
旧r7程序不认识该scope，不能用于打开已执行本次撤权的副本；必须配套包含本契约实现的新Auth程序和CLI。
不能以user_version未变推断可降级，不修改旧r7不可变发行包，也不能删除新事件以迁就旧程序。
升级只作用于获准隔离副本，生产r7及其Auth/Audit未因本任务变动。

纯虚构CLI root/netns过程测试由主线在获准108私有测试区显式执行；107没有通用免密sudo，不因此改其权限。
真实108已完成完整解密/清单、隔离新副本、当前种子绑定、原Audit重确认与持久撤权，并清理本批明文。
真人新登录/L3、活跃Cookie与旧MCP会话重放、新agent正向供给仍未验；不把已完成的副本撤权写成完整灾备完成。

实现：[恢复入口](../../crates/auth/src/session/restore.rs)、[配对审计](../../crates/auth/src/session/restore/audit.rs)、
[CLI装配](../../crates/auth-admin/src/offline_restore.rs)、[本机门禁](../../crates/auth-admin/src/restore_guard.rs)。
