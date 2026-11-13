# 原子过滤封存

`StagingArea` 是专用副本目录的唯一写者。`FrozenOwners::seal` 在原七 owner 冻结窗口中完成实际导出、可信来源登记、过滤、清单与视图原子发布；返回后调用方仍持有冻结窗口。此能力不打开任何 owner 的数据库，不读取备份仓库凭据。

## 真实文件边界

- 已批准 staging 父目录必须私有，源根、ephemeral 与 staging 逐层 nofollow 打开并检查物理祖先，不能只依赖路径字符串。所有根的原 inode 保持固定。
- checkpoint ID 只经 SHA-256 形成 `export-*`、`pending-*`、`sealed-*` 固定名字；不能向名称注入宿主机路径。
- owner 导出到本次新建 `export-*` 的 11 个固定子目录。所有导出文件必须是当前用户的独立普通文件，私有权限；链接、特殊文件、跨设备节点拒绝。
- SourceCatalog 必须来自可信 owner/effective policy。每个实际文件恰好一份登记；缺项、多项、重复或折叠撞名、标签缺失、保留期缺失、声明 revision 与实际流式 revision 不一致均失败关闭。调用方的 observed_revision 不被当成证据。
- `.stversions` 仅在 canonical 根作为规范固定排除；不会依文件名补任意数据标签。ephemeral 不得与任何源根或 staging 根重叠。
- FullObject 独立流式复制；SummaryOnly 必须另交可信脱敏结果及摘要，输出到 `data/summaries/<导出路径摘要>.summary`，不覆盖或伪装原 raw 路径；None/过期/crypto-erased 只保留 inclusion 中的非正文记录。
- 复制前后完整复扫来源，重开逐文件复核目标；保留空目录。固定 256 KiB 缓冲，最多 100 万节点、512 GiB、64 层，写入前留足 2 GiB。元数据序列化过程即施加 128 MiB 单文件上限。

## 发布与再读取

`pending-*` 包含 `data/`、`manifest.json`、`inclusion.json`、`seal.json`。seal 绑定前两份 JSON 的实际摘要；inclusion 覆盖全部目标文件摘要/大小、目录和原始来源判定。文件刷盘后变为 0400，目录刷盘后变为 0500。

发布前必须回收含排除 raw 的原始 export；清理只沿本次新建目录的原 FD，拒绝根身份替换，unlink 不跟随链接。失败/展开也回收本次 export 与 pending，无法完成的 IO/身份异常不会改删其他目录；异常残留须由受控回收程序处理。只有 Linux 的原子 NOREPLACE rename 加父目录 fsync 成功，且最终目录重开复验成功，才返回 `SealedReference`。没有覆盖旧尝试或扫描目录选“最新”的行为。

worker 只能接受原引用：checkpoint ID 与 seal 的外部摘要。`SealedCheckpoint::open` 重开固定目录、检查只读权限与精确布局、核 manifest/票据一致性、重新哈希所有内容，并验证保留期与同一剩余期限。原始导出、清单计算和文件复验耗时不能重置全局冻结期限或传输副本保留期。

## 验证与未接线范围

真实文件契约覆盖过滤/独立 inode/只读权限/内容篡改/多余文件/链接/硬链接/缺元数据/revision/过期/错误引用。实际六 owner + 双 Audit 集成在整个封存及失败回收期间检查 SQL 屏障和两端语义门禁；Bridge 明确 absent，不代替真实 cgroup 平台演练。

生产来源登记仍须接可信元数据提供方；当前 TaskEngine 输入只存 restricted 分类，不是完整 DataLabels，测试里的 Full fixture 工厂不得用于生产默认分类。实际 startup/调度、独立仓库 worker、远端最终清单提交与完整新根恢复仍在 T147/T204 后续写域；本文件不表示整条 Rust 备份链已经完成。
