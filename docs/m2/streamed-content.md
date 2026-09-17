# 流式附件、L0与恢复

T-046，2026-09-13。Core/Broker源码已整合，严格Clippy通过，运行契约正在验收。
本页不证明真实Sync额度、附件上传入口或设备往返已接通。

## 固定入口

`RevisionHasher`与原`Revision::compute`使用同一算法和域；Vault UUID、路径和正文
都属于revision。原始SHA-256与路径绑定revision分别保存，不互相替代。

`BlobSource::open`只固定显式绝对根下的规范逻辑文件；不暴露FD、任意Read回调或
可写stage。来源根、各级父路径及文件的inode、ctime、mtime、大小在读取前后复核。
`identity`计算完整字节身份，调用者仍须持有实际来源与目标范围的批准。

`prepare_stream_mutation`接收路径、CAS条件、原幂等键、精确字节身份、来源及显式
上限。返回的`PreparedStream`不可构造、克隆或反序列化；不代表操作已安装。
prepare完成新stage和旧/新L0的流式复制、哈希与同步。唯一owner最后复验身份、
范围和配置代次后，才同步调用`commit_stream_mutation`。

commit先检查已封存stage、旧canonical及旧/新L0证明，再持久Installing、安装、同步父目录、
分块复核实际结果并持久Completed。进入Installing后的错误保持未知；相同hash、
当前路径或重开观察不能补成原请求成功。确切已提交终态才允许原键重放。

## 容量与兼容

- 固定64KiB读取缓冲；内部单对象256MiB；新输入还受显式effective_limit约束。
- staging包含孤儿共最多64项、512MiB；唯一可变Broker与持久单操作槽限制并发。
- 复制以预留的精确expected.bytes为硬限，多读一字节只用于拒绝，不写入stage。
- 旧canonical、历史及协调证据保全使用内部上限，不随新上传effective_limit下降。
- 普通Vec正文及小正文历史接口保留4MiB上限；大附件不经JSON/base64或整对象Vec。
- L0与snapshots是额外保全输出，不算可回收stage；磁盘不足不能提前清未决证据。

State4支持大对象，后续旧小写与维护启用不能降回旧格式。旧Envelope v1的含正文
指纹保持原算法；流式Envelope v2只接受自身完整受控描述符。原键跨主体、路径、
类型或内容不得复用。旧程序遇新格式必须拒绝，不能把大对象当成旧4MiB语义处理。

## 恢复与维护

`prepare_stream_restore`只从同路径已验证L0选择来源，不接受任意来源路径。
原Restore Intent实际Audit ACK后才允许commit或取消；成功/未知恢复的Outcome
未闭合仍阻写。明确Cancelled/Rejected保留固定补记，沿既有判据不冻结普通编辑。
恢复源后来按保留规则清理，不改变已有原恢复终态的重放。

原未决维护的proposed/current/previous使用同一流式证据机制，不退回大Vec。
维护自身的Intent/Executing/Outcome门禁、实际TTY身份和原Human/Obsidian主体不变。
未知安装仍只可保全协调，不能凭安装后的文件哈希确认成功。

旧State2/3的小请求若遇到外部大current，按实际保全对象选择Manifest v2；完整终态
与证据核验后才在清理原pending的同次保存中启用State4。未引用证据不能自行解锁。

`prune_blob_temporaries`已整合，本机16项专项通过；只处理history/snapshots直接子项中
固定UUID名称的未发布临时文件。每域最多64项/512MiB，Ready且无恢复/维护未决时
整批先验、逐项流式重验再删除，目录每域枚举上限50,001。已发布历史、恢复证据、
日志和快照不入候选；中断返回错误可能已删除部分核验过的临时文件，重开重新审阅。

## 验证口径

新增契约使用完整写入的200MiB+17字节及独立SHA-256黄金值，另在独立子进程测量
峰值RSS，要求低于128MiB以捕获整对象Vec回归。稀疏文件只用于逻辑配额测试。
首次macOS Broker库运行中，上述大文件、内存、旧大文件小保存、严格恢复和流式
协调契约均通过；该次整库仍有两项旧边界失败，不能算整套通过。随后修正旧stage
限额夹具及错误original_content未消费审阅票据的问题，恢复专项32项全过。
独立复核发现的旧/新L0提交前保全缺口已修复，新旧入口10项回归全过，包括缺失、
同长篡改、同内容换inode及最终rename前故障。相关严格Clippy通过。
后续完整workspace运行中Broker库155项通过；其中RSS子测试以ignore隔离入口登记，
由外层测量测试实际单独调用。整套随后在旧writer的4MiB保全假设上失败；
已改为新256MiB上限加一并通过该writer_contract全部30项。Broker锁释放另2项
通过。该冻结点随后完整workspace143目标1540项通过；RSS独立子程序入口的ignore并非跳过
内存测量。另一次静态复核发现旧Vec新写漏检恢复审计未闭合，已补5项红绿回归并通过；
门禁位于原终态重放之后、新写入之前，原恢复可继续确认，其他新写零副作用。
随后的L0 30项、writer/replay 36项、local-server 361项及架构31项全部通过。
Linux新增链路待验；真实断电、磁盘故障和服务协商上限另计。
