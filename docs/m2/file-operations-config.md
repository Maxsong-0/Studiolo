# 文件操作的私有批准

T-046，2026-09-13。配置模块已实现，本机7项配置契约及1项compile-fail通过。
当前Broker删除/墓碑协议与实际维护/网页接线仍在实施；配置存在不代表已经启用服务。

## 明确关闭与批准来源

既有私有主人配置版本1新增可选对象file_operations。省略表示关闭；null、布尔、数组、
未知、重复或缺字段一律拒绝。只有human_content存在且Vault UUID完全一致才能批准。
不允许HTTP请求提供开关、配额、范围摘要或可信主体；也不根据用户给的远端库名启用。

| 必需字段 | 固定约束 |
|---|---|
| version | 1 |
| vault_uuid | 与同文件human_content一致 |
| scope_epoch | 1..=i64::MAX；真实持久回退检查另由owner负责 |
| effective_object_limit | 1..=268435456字节；不是Sync账号配额 |
| max_tombstones | 1..=4096 |
| retained_bytes_limit | 1..=2199023255552字节 |
| minimum_free_bytes | 非零；实际设备容量由Broker逐次核查 |

完整配置仍受16KiB与原0600/0700、UID、普通单链接文件、稳定父目录约束。
容量字段限制实际操作，不能据配置能加载推断已有空间；它不包含永久删除授权。

## 整文件保护

FileOperationPolicy只从原Config Snapshot构造，CanonicalServingRoots保留它。
每次allows前后复核原配置，并要求原人写范围批准和同一Vault身份。隐藏路径、raw、
系统保留路径及INDEX/TASKS/dashboard等整页不可破坏；显式列入人写documents也不能
覆盖此规则。protected和非空generated区域的登记按保守Unicode大小写别名排除自身及后代。
生成哨兵已缺失仍不能整页删除或恢复；原部分编辑allows/preserves_generated语义不变。

范围摘要包含独立域、原始配置长度及整个原文件字节。预算、owner、无关字段或格式变化
使旧摘要失效；同字节换inode也不能继续使用原批准。文件操作owner仍须使用实际Broker
的Vault UUID/根身份，并在最后提交前复核真实会话、配置与范围，不能只比HTTP自报值。

## 后续装配边界

默认关闭的批准对象不创建目录、初始化存储、修改配额或清理墓碑。实际enable、操作和
至少7天保全由[文件操作协议](file-operations-design.md)负责。
新的文件操作审计投影使用schema2，完整Broker v1事件放在file_operation字段；
保持原正文schema1和旧维护事件字节不变。真实Ledger完整回执由可信owner核验后ACK。
