# 原子过滤封存

`StagingArea` 是专用副本目录的唯一写者。`FrozenOwners::seal` 在原七 owner 冻结窗口中完成实际导出、可信来源登记、过滤、清单与视图原子发布；返回后调用方仍持有冻结窗口。此能力不打开任何 owner 的数据库，不读取备份仓库凭据。

## 真实文件边界

- 已批准 staging 父目录默认私有；专用 worker 交接使用下述 0750 读取组布局。源根、ephemeral 与 staging 逐层 nofollow 打开并检查物理祖先，不能只依赖路径字符串。所有根的原 inode 保持固定。
- checkpoint ID 只经 SHA-256 形成 `export-*`、`pending-*`、`sealed-*` 固定名字；不能向名称注入宿主机路径。
- owner 导出到本次新建 `export-*` 的 11 个固定子目录。所有导出文件必须是当前用户的独立普通文件，私有权限；链接、特殊文件、跨设备节点拒绝。
- SourceCatalog 必须来自可信 owner/effective policy。每个实际文件恰好一份登记；缺项、多项、重复或折叠撞名、标签缺失、保留期缺失、声明 revision 与实际流式 revision 不一致均失败关闭。调用方的 observed_revision 不被当成证据。
- `.stversions` 仅在 canonical 根作为规范固定排除；不会依文件名补任意数据标签。ephemeral 不得与任何源根或 staging 根重叠。
- FullObject 独立流式复制；SummaryOnly 必须另交可信脱敏结果及摘要，输出到 `data/summaries/<导出路径摘要>.summary`，不覆盖或伪装原 raw 路径；None/过期/crypto-erased 只保留 inclusion 中的非正文记录。
- 复制前后完整复扫来源，重开逐文件复核目标；保留空目录。固定 256 KiB 缓冲，最多 100 万节点、512 GiB、64 层，写入前留足 2 GiB。元数据序列化过程即施加 128 MiB 单文件上限。

## 发布与再读取

`pending-*` 包含 `data/`、`manifest.json`、`inclusion.json`、`seal.json`。seal 绑定前两份 JSON 的实际摘要；inclusion 覆盖全部目标文件摘要/大小、目录和原始来源判定。文件刷盘后变为 0400，目录刷盘后变为 0500。

发布前必须回收含排除 raw 的原始 export；清理只沿本次新建目录的原 FD，拒绝根身份替换，unlink 不跟随链接。只有 Linux 的原子 NOREPLACE rename 加父目录 fsync 成功，且最终目录重开复验成功，才返回 `SealedReference`。没有覆盖旧尝试或扫描目录选“最新”的行为。

同一父目录以固定 `.producer-lease` 空文件的内核排他锁限定一个生产者；锁为当前生产者所有、0600、单链接、无 ACL，原 inode 每次复核，正常流程从不删除锁文件。复制正文前先创建并同步 `incomplete-<checkpoint 摘要>` 标记。构造未完成时的错误、显式 `abort`、封存失败和展开都会尝试回收本次原 export/pending；rename 已生效但 fsync 或后续复验失败时，还核原 inode 回收本次 sealed。已经部分或全部改成读取组权限的根，也只能由原生产者按固定读取组回收；不扩展 worker 的删除权限。

失败回收最多 30 秒，并受原冻结/封存截止时间进一步限制，不能为清理重置窗口；每个原根最多 100 万节点及 64 层，先收回目录组可读性再删除。只有全部原目录确认回收才移除未完成标记。截止已到或无法确认时返回 `CleanupFailed`，保留原登记，待调用方释放业务冻结后由独立回收入口续办；下一作业返回 `CleanupRequired`。进程被杀后即使内核锁已释放，也不能越过未完成标记；旧版本遗留的 export/pending 同样阻塞。

未完成目录先写原父目录/标记目录身份，再逐项登记 export/pending 的设备号和 inode，登记持久之前不写该目录的正文。日志仅允许 `identity.json`、`export.json`、`pending.json`、`sealed.json` 与对应 `.new`；每文件最多 16 KiB，先同步 0400 临时文件再 NOREPLACE 发布。完整封存复验后，保存原 SealedReference 及原 seal 的规范字节，期限必须从该摘要绑定的 seal 推导，不能另填一个到期数字。原标记最后原子改名为 `retained-*`，再返还引用。

`reclaim_staging` 是生产者的受控回收入口：只打开已存在的固定租约，不初始化任意目录；读取真实系统 UTC，共用最多 30 秒单调截止。一次最多核验 256 份登记，完整核对全部目录与日志后才删除。未完成登记按原 inode 回收；retained 只有到期后回收，未到期返回最近期限供服务再次唤醒。每次删除前重核原标记与全部日志字节。根已删除但标记尚未清除、或清理途中原 seal 文件已删时，仍用持久原 seal 证明和原目录身份续办。无登记、日志损坏、未知节点、根/组替换，或时钟早于登记时刻/本次回收起点，均失败关闭，不按名字猜测所有权。服务启动和定时器还须实际调用此入口，不能把 API 本身当作已运行的自动 GC。

读取只接受原引用：checkpoint ID 与 seal 的外部摘要。`SealedCheckpoint::open` 重开私有固定目录、检查只读权限与精确布局、核 manifest/票据一致性、重新哈希所有内容，并验证保留期与同一剩余期限。原始导出、清单计算和文件复验耗时不能重置全局冻结期限或传输副本保留期。

## 独立身份的只读交接

`create_for_reader` 要求部署预先建立生产者 UID、专用 worker 读取组 GID 的 0750 父目录。它不创建系统账号、不修改已有父目录权限。原始 export 和 pending 保持 0700；发布并完整核验私有封存后，逐个原 FD 将过滤视图文件改为 0440、子目录改为 0550，组固定为已批准 GID。根仍为 0500，再完整核验一次全部元数据和内容；只有通过后才把根改为 0550 并返回引用。

读取组无写权，worker 使用不同非 root UID。`open_for_worker` 从受保护部署配置接收 `ReaderIdentity`，拒绝同 UID/root 冒充独立身份，每次读取核生产者 UID、读取组 GID、精确权限、原父目录/根身份和全部字节；不能把请求自报的 UID/GID 当成配置。`open_shared` 仅供生产者恢复自己的待交接记录，其生产者 UID 固定为当前系统身份。

Linux 上私有和共享节点都拒绝 POSIX access/default ACL，避免 chmod 改 mask 时把继承的命名用户重新放开；不支持此权限验证的平台拒绝启用该文件能力。只读组权限不代表 NAS 不可变性，也不限制生产者自身重新 chmod；实际 worker 仍须在传输前后核原引用，并验证远端完整内容。

## 验证与未接线范围

真实文件契约覆盖过滤/独立 inode/只读权限/内容篡改/多余文件/链接/硬链接/缺元数据/revision/过期/错误引用，以及共享父目录、错误组、同 UID worker、access/default ACL 拒绝。实际六 owner + 双 Audit 集成在私有和共享封存及失败回收期间检查 SQL 屏障和两端语义门禁；Bridge 明确 absent，不代替真实 cgroup 平台演练。测试箱仅映射一个 UID，未把单 UID 的权限检查冒充两个生产账号的实际读取演练。

生产来源登记仍须接可信元数据提供方；当前 TaskEngine 输入只存 restricted 分类，不是完整 DataLabels，测试里的 Full fixture 工厂不得用于生产默认分类。原生独立执行器与远端最终记录机制已实现，见 [最终记录契约](backup-finalization.md)。实际 startup/调度、审计 IPC、回收定时器与完整新根恢复仍在 T147/T204 后续写域；本文件不表示整条 Rust 备份链已经完成。
