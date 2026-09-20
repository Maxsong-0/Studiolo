# 可信本地因子管理与离线撤权命令（T-017 / T-094）

> 本文保留 T-092～T-094 初次验收时的部署/提交状态。后续生产程序发布使用人侧 release 工具；历史恢复回执不会因发布而变成完整灾备验收。

T-017在2026-09-05完成双平台契约验收；下文历史结果不代表当前部署状态。
T-094离线撤权在2026-09-20修复后107原生CLI测试34项、108 root未来时钟夹具1项通过；
真实108副本已验证v2绑定、epoch推进、旧L3/10枚恢复码清除、原时钟/预算保留与幂等审计。
新能力未部署生产；恢复副本真人重新登录/L3未验。详见[实际回执](../../deploy/human/offline/ACCEPTANCE.md)。
API与程序兼容边界见[离线恢复契约](auth-restore.md)。

## 入口与信任边界

独立的`studiolo-auth-admin`是单用途本地装配，不把环境/终端权限加入Auth。
没有HTTP、localhost授权、首次抢注、清库重登、移除因子或丢失全部因子的恢复入口。
不生成种子/二维码，不签发AccessIdentity、Cookie、会话或L3，不启动子进程。
TTY仅保护输入，不隔离同UID或管理员。目录须由部署预先批准且父目录可信。
没有Vault参数，不能自行证明部署目录位于所有真实Vault之外。

```text
studiolo-auth-admin migrate --config /approved/private/owner.json --from 3
studiolo-auth-admin enroll --config /approved/private/owner.json
studiolo-auth-admin audit-flush --config /approved/private/owner.json
```

- 上述三个交互命令要求stdin/stderr同一前台控制TTY；stdout可重定向，只输出非秘密固定结果。
- 普通登记/补记真正no-create打开已有Auth/Audit主库；缺失时拒绝，不悄悄建立空库。
- 明确`migrate --from 1|2|3`调用既有显式步骤；新库可用3直接建v4，已有v1/v2按原步骤升级。
  不创建目录、修权限、清坏库、改主人或改变旧有效期。迁移不登记、不撤销已有因子。
  多次迁移及Auth/Audit两库初始化不是跨库原子事务，可能留下合法中间版本。
- `TotpEnrollment::state()`仅显示Ready/Pending/Enrolled，不消费、不补记、不清pending、不授权。
  已登记明确拒绝，不冒称本次成功；Pending不索取OTP，须按补记/到期规则处理。
- 真实登记沿原TOTP预算、消费、双审计与原调用最终CAS；补记仅确认原事件，绝不自动登记。

## 非秘密配置

版本1、拒绝未知/缺失/重复字段，最多16KiB。文件要求绝对路径、0600、当前UID、单硬链接、普通文件；
T-019将该只读配置Implementation提取到共享`studiolo-host-config`，供管理命令与显式会话启动复用；
JSON结构与登记流程保留，字段改为封闭getter；[启动契约](auth-startup.md)另外约束公开根隔离。
配置父目录与Auth/Audit根要求预先存在、0700、当前UID。所有路径祖先拒绝符号链接。
拒绝setuid/setgid进程、两存储根相同/嵌套及可识别的真实目录身份别名。
读取用NOFOLLOW/NONBLOCK/CLOEXEC、大小上限及打开前后对象身份检查；不扫描/修复其他兄弟文件。

以下是非秘密结构示意；身份、目录和期限必须由主人明确批准，不能直接当生产配置：

```json
{
  "version": 1,
  "auth_directory": "/approved/auth",
  "audit_directory": "/approved/audit",
  "owner": {
    "issuer": "https://YOUR_TEAM.cloudflareaccess.com",
    "audience": "YOUR_APPROVED_AUDIENCE",
    "subject": "YOUR_APPROVED_SUBJECT",
    "email": "owner@example.invalid"
  },
  "idle_seconds": 1800,
  "absolute_seconds": 43200,
  "seed_ref": "owner-totp-v1"
}
```

Owner/TTL使用原Auth构造约束，重开必须与持久配置相同。seed_ref只是1～64位规范ASCII标识，不是路径。

## 交互命令的秘密与终端

- 种子只读固定`STUDIOLO_TOTP_SEED_HEX`：恰好40位小写hex，解码为20字节；无默认值/回退生成。
  由已批准的环境注入机制提供，不能放在命令参数、普通配置、shell历史、聊天、日志或审计。
  不自动加载`.env`或读种子文件；本片不自主创建secret mount。手机预配置和真实种子交付仍属外部安全流程。
- 自有环境副本/解码缓冲进入零化封装，不宣称擦除OS/父进程环境、调用方副本或第三方所有内部缓冲。
  在读取秘密之前把当前短命进程的核心转储软/硬限额均降为0并核读，不改父进程或服务；不冒称抵御管理员内存读取。
  （**2026-09-17 T-056 核实**：该收紧现已是四项封闭措施，不止核心转储——`CoreDumpLimit`、
  `ProcessInspection`（关可转储属性，挡 ptrace 与同 UID 读 `/proc/<pid>/{environ,mem,maps}`）、
  `MemoryResidency`（锁定页不换出交换区）、`EnvironmentSlot`；见
  `crates/host-config/src/process_safety/report.rs`，入口
  `studiolo_host_config::process_safety::harden()` 由 `main.rs` 的 `run()` 第一条语句调用。
  （**2026-09-17 T-059**：该实现已从 `crates/auth-admin/src/process_safety*` 搬到
  `studiolo-host-config`，由登记工具、`studiolo-local-server` 与 `studiolo-maintain` 共用
  同一份；本 crate 不再自持任何一份部分收紧，守卫见
  `crates/auth-admin/tests/process_discipline.rs`。）规则是两分的：本平台**提供**的能力必须真正
  生效并回读确认，否则整体拒绝且不读取秘密；本平台**不提供**的能力记为明确的「不支持」分类，
  由调用方（`main::run`）负责记录下来——只返回不记录等同于静默跳过，同样不允许。
  本条只收紧本短命进程自身，仍不抵御 root、调试授权持有者或已能读本进程内存的一方。）
  产品不派生子进程，不提供任意环境变量名配置；不在运行中修改环境来伪称彻底清除。
- OTP从控制TTY隐藏读取固定6位ASCII数字；逐字节有界缓冲，支持退格，拒绝第7位、非ASCII、EOF与取消。
  隐藏模式设置后才提示输入；退出前丢弃多余输入，显式恢复并核读属性。
- 输入期间安全捕获SIGINT/SIGTERM/SIGHUP/SIGQUIT/SIGTSTP，恢复后再启用默认行为。
  SIGKILL/SIGSTOP、终端实际断开与不可恢复系统故障无法承诺恢复；不得把Drop尽力清理当作成功。
  断开后可能先返回取消、I/O或恢复错误，再被迟到SIGHUP按已恢复的默认行为终止；非零退出不代表返回过验证码。

## 交互命令的失败与重试

配置/TTY/种子预检在数据库装配前；进入合法数据库后，即使OTP错误或取消，也可能推进时间地板/到期状态并产生SQLite侧文件。
`open_existing`不是只读句柄，只保证不创建缺失主库，不绕过原权限/格式/绑定/审计回执检查。

审计失败可能已消费OTP并形成pending，但不登记。补记只送原事件；双ACK后仍未失效的pending继续保护。
失效且安全释放后才重新用新OTP登记，持久限流与消费高水位不重置。
`audit-flush` 依次补记 TOTP、v4 Recovery 和最多256条会话／拒绝事件，可能需再运行。
v3没有恢复表，继续补原TOTP与会话事件而不隐式迁移；完整恢复维护须先显式升级v4。
恢复补记不交付旧恢复码、不激活新批次、不切换因子、不撤销会话或授予L3。
尚未失效的pending即使双ACK也继续保护；只有失效且双ACK的原操作可安全释放。
恢复成功后须先把批准的配置和秘密注入对齐到当前权威因子，旧绑定仍明确拒绝；
补记不会自行选择、生成或替换秘密。某阶段失败时之前阶段可能已提交，不承诺跨库整批原子。

提交后存储复核失败或stdout输出失败可能“非零退出但操作已提交”。须修复外部条件并核对状态，不能自动重放/清库。
再次登记若已完成，会在索取OTP前明确AlreadyEnrolled；这不是再次执行成功。

## 隔离副本的离线撤权（T-094）

这是独立的可信本地装配入口，保留当前已登记因子，使副本内旧会话、L3与恢复码失效。
不提供HTTP入口、因子丢失救援、重新登记、自动恢复服务或生产撤权流程。
外层可信包装器须先校验完整备份清单、复制到新目录、冻结全部副本写者并阻止服务监听；
CLI不证明数据来自合法备份，也不防御有权改动这些root私有目录的管理员。

```text
studiolo-auth-admin restore-invalidate --config /var/lib/studiolo-dr-tests/RUN-ID/human-work/owner.json --restore-root /var/lib/studiolo-dr-tests/RUN-ID/human-work --restore-id RESTORE-ID
```

参数按上述固定顺序提供。`RUN-ID`为1～64位、`RESTORE-ID`为1～55位ASCII字母数字或连字符；
均为非秘密标识，未知/重复/缺失参数拒绝且不回显。stdin由获准的私有管道提供，不把种子填进此命令。

- 仅Linux，真实与有效UID均为0；stdin必须非TTY，当前网络命名空间必须不同于`/proc/1/ns/net`。
  `studiolo_process_safety::harden()`仍在参数、配置及秘密读取之前执行并输出固定分类到stderr。
- restore-root必须逐字匹配固定绝对布局，拒绝别名、`..`、额外层级及生产路径。
  从`/`起全部祖先要求root所有、无符号链接和组/其他可写位、同一设备；固定布局不允许sticky放宽。
  `/var/lib/studiolo-dr-tests`、run目录、human-work、auth与audit均须root0700且预先存在。
- human-work仅含`owner.json`、`auth`、`audit`。配置必须精确为该owner.json；
  `auth_directory`/`audit_directory`必须逐字等于其下auth/audit，`seed_ref`必须为`studiolo-human-totp-v2`。
  owner.json、auth/auth.sqlite3、audit/audit.sqlite3均须root0600普通单链接文件，主文件非空。
  两库只额外允许固定同名`-wal`/`-shm`/`-journal`普通单链接root0600侧文件；不接受其他条目或跨设备。
  目录与主文件持有打开的FD，并在装配、撤权前后重验身份及权限；配置还校验大小和修改/变更时间。
- stdin恰40字节小写hex后立即EOF；拒绝换行、空白、大小写变体、超长及短输入。
  输入最多读取41字节并由`Zeroizing<[u8;41]>`持有；本分支不读取`STUDIOLO_TOTP_SEED_HEX`。
  不使用TTY验证码，不输出种子、OTP、恢复码、token、身份或配置正文。
- 使用原`Config`、`SqliteAuditLedger::open_existing`与`SessionManager::open_existing_for_restore`；
  不创建/迁移主库。`Totp::attach`核对v2种子绑定，`Recovery::attach`确认v4；
  `SessionManager::invalidate_restored_credentials(&restore_id)`直接检查已有登记证明并重核原审计。
  离线路径不调用`TotpEnrollment::state()`，因为它会观察运行时钟并推进时钟地板/到期状态。
  专用Auth打开入口仅ExistingOnly/MigrationNone/v4，不执行`SessionManager::run`，不观察或推进时钟/到期状态。
  普通`open_existing`仍保持原运行检查，不能用于此离线装配；SQLite侧文件或撤权事务本身仍可能写入。

stdout成功返回一行固定非秘密JSON；示例数字仅为结构示意：

```json
{"epoch":2,"revoked_sessions":1,"cleared_l3":1,"invalidated_recovery_codes":10,"audit_confirmed":true}
```

只有`audit_confirmed=true`且输出/最终复核成功才退出0。`audit_confirmed=false`时仍输出上述五字段JSON，
随后以非零退出；撤权可能已提交，必须保持离线，先核对原恢复ID及账本，不能生成新ID盲重试。
同ID会复用原epoch和计数并重核证明/原审计事件，详见[恢复API语义](auth-restore.md)。
本命令成功不等于新会话、L3、恢复码签发或服务可上线授权；原三个命令的TTY行为不变。

## 构建与验证边界

```text
cargo --locked --offline build -p studiolo-auth-admin --features production
cargo --locked --offline test -p studiolo-auth-admin
```

production显式转发Auth生产特征，不默认启用以避免workspace测试特征融合；与test-support混用必须编译失败。
测试只用虚构种子和临时私有双SQLite；Python3仅为真实PTY/产品进程验收依赖，不允许缺失时静默跳过。
本契约不等于真实因子登记、HTTP接线、密钥部署、恢复码保管、服务器原生或完整M2验收。

T-017最终：macOS475/Linux aarch64 483（各含3项身份compile-fail），新增54项；CLI29单元+1实际进程，
其中真实PTY契约驱动15种场景，不重复累计为15个Rust测试。两端格式/严格Clippy、生产正反构建通过，
macOS release及Linux dev生产二进制另跑真实CLI契约。实际不同UID/setuid、commit I/O/断电和真实主人材料未验。
macOS退出隐藏输入时的PENDIN过渡依[XNU实现](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/tty.c)
与实测位差，仅允许一次额外完整恢复，不删除最终属性检查。
