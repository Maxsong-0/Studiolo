# 整文件同步范围

T-046，2026-09-13。私有配置与范围核对已实现，本机38项契约通过。
这不是实际SyncTransport；尚未据此扫描或同步真实Vault。

## 批准来自哪里

`Config::sync_scope()`只从既有0600私有配置取得封闭`SyncScope`。缺少`sync_scope`
字段表示关闭；显式null、数组、重复/未知字段或不完整规则均拒绝。原配置版本1
和16KiB上限不变，不自动初始化数据库或扩展浏览器的人写范围。

新增对象version固定1，含canonical `vault_uuid`、非零32位小写十六进制`link_id`
和`remote_vault`、正整数`scope_epoch`、`whole_folders`、`whole_files`及
`generated_documents`。远端身份必须由实际接入核对，不从显示名称猜测。
目录规则最多64项，精确文件128项，生成文档128项；批准根不能重叠。

每条规则显式指定`markdown`、`attachment_types`和`max_bytes`，至少批准一种类型；
单文件上限1..=268435456字节。附件闭集为png/jpeg/gif/webp/pdf/mp3/mp4/wav/zip/txt/json
（txt／json 自 2026-09-23 起加入，ADR-025 D12：`_规则/归档/` 的纯文本走目录规则显式批准，
headless 侧它们归入 `unsupported` 类）。
参数是内部策略上限，不能冒充真实Sync服务配额；文件后缀不自动授予类型批准。

绑定摘要包含域分隔、实际完整配置长度与原始字节；修改配置中任何字段或格式都
使旧摘要失效。inode、链接数、权限及私有父目录变化也会使旧Snapshot失效。
`scope_epoch`的持久回退检查由后续状态owner负责，当前配置解析不证明历史单调性。

## 读取与完整批准

`policy_for_observation(binding,path)`在读取正文前返回只读类型/字节约束。
None表示该路径正文不获批准；错误不转成None。它不证明路径存在/缺失、正文类型、
摘要或扫描完整，也不是目录递归判定。配置在检查前后重验。

`approve_union`核对旧共同基线、canonical和镜像三组完整事实。双方当前观察必须
显式覆盖完整联合路径；Missing用None表达，不能用漏项表达。包括旧基线在内任何
未批准路径、类型变化、碰撞、超限或漏项都会拒绝整批，无部分批准。

所有三方合计最多4096个不同路径，联合路径文字1MiB，逐路径1024字节/64层；
派生前缀最多16384项/4MiB，三方实际Present字节合计不超过4GiB。
大小写、NFC、保守Unicode大小写别名、文件与子路径冲突一并检查。

## 排除优先与产品边界

隐藏路径、raw、ops、sources、policy、skills、capabilities、私有根，以及
TASKS/INDEX/dashboard/CLAUDE/AGENTS/SKILL保留文档不可由宽目录批准覆盖；
`_规则/` 自 2026-09-23 起可由目录规则批准（人侧网页保存仍保护，见 T-160），
其下的保留文档名、AI 专属目录与隐藏路径照样拒绝。
既有人写范围的protected/generated登记与新增生成文档优先于批准；删掉生成哨兵
不会把文档变成整页可同步内容。混合TASKS页的人写条目回写需要独立协议，当前
不把整页复制冒充该能力已实现。

真实读取器仍需证明安全根、完整枚举、真实字节类型、流式摘要、显式Missing与
批尾稳定性，再交给完整批准；当前工作台capture不满足附件同步观察契约。
受控镜像进程、双向搬运和冲突保全仍见[传输设计](sync-transport-design.md)。
用户接入信息见[查找步骤](connection-setup.md)，无需自行填写内部身份或摘要。
