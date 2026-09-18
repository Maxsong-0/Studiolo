# 人侧保存的后续设计输入

> **状态更正（2026-09-17 T-056 文档漂移审计）**：本页写于 2026-09-05，是设计输入。
> 此后 T-046 已把它实现：可信可编辑范围 `HumanContentScope`（含 `allows`、
> `preserves_generated`）见 `crates/host-config/src/human_content.rs:10,99,168`，
> `POST /api/v1/mutations` 的 replace 分支见 `crates/local-server/src/editing.rs:19`
> 并在 `routing.rs:243-246` 装配。**下文「尚未实现」「host-config 尚无可编辑范围」
> 只反映写作当日**；现行权威是[人侧正文编辑契约](human-content-contract.md)，
> 整文件删除/恢复/重命名另见 [`file-operations.md`](file-operations.md)、
> [`file-rename-contract.md`](file-rename-contract.md)、[`human-rename-contract.md`](human-rename-contract.md)。
> 仍然成立的是：真实身份与生产发布未验收。

2026-09-05，只读核查；T-028及新鲜度之后另行认领，尚未实现。
普通内容保存沿矩阵L2，规则母本/skills/能力集归M3的L3 policy/activate。
有效L3也不能让普通保存端点兼任策略发布；AI整页替换不属于此入口。

## 已知分类缺口

core::VaultPath::is_protected只处理根级.git/.workbench/.obsidian及.stignore，
不等同保存权限分类。真实权威档案指向`_规则/通用工作规则.md`，同处还有SKILL.md；
规则快照CLAUDE.md、ops/agents.md与不可变产物Markdown也能通过底层保护检查。
不能据此把所有非隐藏Markdown自动归为正文。

权威目前没有完整的策略路径注册表，也没有所有生成区sentinel的字面格式；host-config
尚无可编辑范围。后续须由可信装配提供HumanContentScope，不能从文件里的type、
editable:true、标题、扩展名或是否被索引推导写权限。动态扩权不能走L2正文登记。

## 最小replace入口

继续使用已登记POST /api/v1/mutations，仅接受封闭replace分支：kind、vault_uuid、
path、expected_revision、content、idempotency_key。无query、force、客户端主体等级、
MustNotExist或ignore_stale；不直接反序列化内部Mutation/Precondition。

- 真实Access和主人L2配对，CSRF/来源/正文与头预算；动作执行时在writer owner内重验。
- 已有普通UTF-8 Markdown、可信内容归属、完整新鲜度、绑定/未决与实时目标CAS均通过。
- 写入侧frontmatter明确校验；读取容错不等于写入许可，正文不进入审计payload。
- 人侧内容沿“审计不可用仍可执行”的矩阵；不套用策略/AI动作失败关闭规则。
- 冲突保留双方，不自动覆盖；回执描述历史提交。未知结果不自动产生另一次保存。

可信分类必须优先保护规则母本/skills/能力集及副本、私有运行路径、黑板/审批/审计投影、
不可变attempt产物/manifest/raw和.obsidian/.workbench等对象。未知角色先保持只读，
启用前必须有明确的普通正文范围及不可被正文覆盖的保护登记。

## 不得静默缩减的后续范围

首个replace切片可以先排除新建/删除/重命名/附件，但完整同步与编辑需求仍保留。
dashboard、INDEX、TASKS的混合生成区需要可信登记及旧新内容逐字节保护；首片若未支持，
必须显示不可整页保存并登记缺口，不能把所有同名人写文件永久归为系统文件。
ADR-010的人写工作项权威仍成立，后续须允许保留生成区的人写部分编辑。

正式实施前明确：普通正文范围和策略保护登记、混合文件及sentinel格式、未知路径的受信
分类流程。该设计不是允许发布缩减M2，也不以猜测的文件名黑名单代替授权。
