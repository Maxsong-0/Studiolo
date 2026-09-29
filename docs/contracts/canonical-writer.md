# canonical单写者与幂等（T-028）

已通过macOS724/Linux735全工作区及两端生产检查，实际证据见[M2记录](../m2/README.md)。本片不提供HTTP编辑或同步，
不将持久水位直接称为索引新鲜度；[窄读取](canonical-read.md)继续独立使用。

## 唯一拥有者

FsVaultBroker每实例独立打开canonical根目录可读FD，取得非阻塞独占flock并持有到销毁。
只在最后FD销毁时释放，不用任意data目录锁或可替换marker锁。一次实例失败不会解开
其他实例的锁。底层目录探针及macOS/Linux产品的同/跨进程争用与释放契约均通过。

canonical与data不重叠，永久配对绑定Vault UUID及目录身份。双方分别保留
`.workbench/broker-binding.json`与`broker-binding.json`，单边缺失或不匹配不能由
普通打开修复。更换data目录不能绕过历史回执、未决状态或提交水位。

根、`.workbench`、身份marker与data对象被钉住并在使用时复核；同UUID的marker替换
也失效。保留符号链接、硬链接、特殊节点拒绝，根FD锁不作为恶意同UID进程的隔离边界。

## Interface

- `FsVaultBroker::open(vault_root,data_root)`保留显式初始化/采用用途；采用旧存储前必须
  有界验证旧marker和回执，不能忽略未知未决，也不能补齐一份不明来源的单边绑定。
- `open_existing(vault_root,data_root)`只打开完整已有布局，不创建缺根、身份、绑定或
  存储目录；未来生产组合必须显式使用它。正常已记录操作的恢复与新的写入仍由此写能力承担。
- `write_status()`仅返回`WriterStatus { committed_through:u64, unresolved:bool }`，
  无原始日志、stage路径、正文或授权票据。状态本身不证明某个HTTP请求已交付。
- `Mutation`、`Precondition`、`MutationReceipt`与Revision算法保持；完成回执只描述
  历史提交，不保证文件当前仍为该版本。

稳定新错误：WriterBusy（租约占用）、WriterBindingMismatch（配对不符）、
CorruptWriterState（非法持久状态）、MutationUnresolved（安装不可确认）、
WriterLimitExceeded（协议预算）。未知安装不能冒充普通CAS冲突或成功。

## 持久阶段与历史

data的固定`writer-state.json`只容纳一笔在途操作和提交水位；按key保留终态记录。
自有stage位于canonical的`.workbench/staging/`，不进入Sync镜像；外部请求不能指定
stage或任意环境路径。记录格式和预算是私有Adapter协议，不开放原始JSON写接口。

| 阶段 | 后续允许的行为 |
|---|---|
| Prepared | 尚未安装；绑定有效且原请求重验CAS后才可继续，必须先持久化Installing。安全取消形成明确拒绝，仅清理身份匹配的自有stage。 |
| Installing | 安装结果不可证明；保持未决，禁止任何新canonical安装越过。当前内容相同、stage消失或inode相同均不构成成功证明。 |
| Completed | 原调用已完成安装、校验和父目录同步；重开只补终态回执与水位，不重装文件。 |

首次明确CAS拒绝保持拒绝语义。T-028在旧实现上已真实复现：错误前置B要求A→C被拒绝，
另一key合法A→C后，重放原key被误补成功；同进程和重开均存在，Memory对照仍拒绝。
修复必须保留这两条失败复现作为正向拒绝回归，不能仅把首次CAS提前而忽略安装中断。

旧内容以实际读取revision保存并同步L0历史，再临时文件同步、原子安装、父目录同步。
已完成重放不重装、不重复推进水位。artifact各canonical安装与manifest也须纳入同一
序列；后续bridge/显式恢复必须遵守该规则，才能让新鲜度以完整水位为依据。

## M0兼容与未决

旧`receipt=None`缺少阶段证据，一律未知未决，不能转换成Prepared；采用时有界枚举，
在解除前阻断所有canonical写入。旧完成回执可保留，但不改写成新协议的已证安装记录。
Installing的显式协调/恢复属后续能力，不能用删除日志、换data目录或相同内容静默解锁。

当前实现预算：marker/绑定4KiB、日志/回执64KiB、mutation/stage/旧内容/历史4MiB，
key4KiB。首次legacy采用枚举最多4096条且合计16MiB；配对后只查固定journal与当前key，
不设新回执累计条数天花板，不宣称启动时完整校验所有历史回执。当前key坏回执必须拒绝。
均须节点校验，超限失败。4MiB是首片内存mutation
边界，不能宣称覆盖真实Vault的大附件同步；后续bridge需要另验有界附件流式写能力。
孤立stage最多64项，超限拒绝新准备；正常成功消耗stage，此限制不是写入生命周期上限。
本片无孤立stage清理或Installing显式协调入口。测试故障点验证持久阶段，不等同真实断电
或提交I/O故障。真实Vault和接入仍需独立验收。

## 路径分量长度（T-132 P6 前遗留）

正文、流式写入及 Delete／Restore／Rename 的源和目标，在持久 Prepared／Preparing 或创建父目录前逐分量按 UTF-8 字节核对文件系统 NAME_MAX。已有父目录读取其自身上限，缺失子树使用最近已有父目录的上限；超限返回 WriterLimitExceeded，不留下未决写状态。VaultPath 的逻辑语法及旧持久请求的解码规则不变。Linux 契约覆盖 255 字节边界、中文多字节超限、缺失父目录和拒绝后重开正常写入。
