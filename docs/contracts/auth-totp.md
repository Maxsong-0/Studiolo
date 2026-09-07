# Auth v3：预配置 TOTP 与近期认证

状态：2026-09-05 T-015双平台契约验收完成。macOS377/Linux385全workspace、格式/严格Clippy与生产正反构建通过；本片新增39项。
补充[会话契约](auth-session.md)和[v2保留](auth-retention.md)，不代表真实登记、HTTP或生产已接线。
动作等级、窗口和失败语义的唯一权威仍是项目档案《敏感动作枚举表》T-015裁决。
T-016增加v4恢复接口与互斥/候选消费高水位，见[恢复补充契约](auth-recovery.md)；下述T-015数字为历史验收记录。

## Interface 与装配

- `SessionManager::migrate_v2(...)` 显式将合法v2增量升级为v3；新目录先v2再v3。
- 普通fresh/open仍保留v2行为；普通open可读取合法v3但不升级v2。v1先沿原`migrate_v1`到v2。
- `Totp::attach(Arc<SessionManager>, TotpSeed)` 接受v3/v4，返回普通`Totp`和独立`TotpEnrollment`。
- `TotpEnrollment::enroll(code)` 仅可信装配使用；成功登记不会给任何会话L3。
- `Totp::step_up(request, code)` 消费正确OTP、审计双确认、原调用重验后为该会话登记近期认证。
- `Totp::require_l3(request, SensitiveAction)` 返回即时检查结果，不签发可缓存凭据。
- `Totp::flush_audit()` 补记两个固定原因子事件，另重确认最多四个登记/旧授权证明；绝不激活。
  返回值只统计本次快照尚未确认的pending事件，最多2；会话拒绝聚合由`SessionManager::flush_audit`交付。

`TotpRequest`始终携带本次新验Access、会话、CSRF和可信入口取得的`IpAddr`，不是已授权DTO。
`SensitiveAction`封闭枚举策略发布（含回滚/启用Adapter）、Vault、备份、Sync、整库恢复、解冻、凭据测试与恢复码管理。
目标Module必须先确认自身intent，再于实际执行/最终切换前检查；排队后重验，不把检查与外部副作用冒称原子事务。

种子由独立secret mount/等价机制的可信装配提供，不由Auth读取环境或任意文件。
`TotpSeed`固定20字节，引用为1～64位ASCII字母数字/点/下划线/短横线，不解释成路径。
AuthDB只存引用和域分离SHA256绑定摘要，引用背后换种子也拒绝；普通请求不能挑选引用或替换因子。
具体内存封装作为当前依赖，不造假想KMS Port。它只零化自己拥有的缓冲区，不保证调用者副本或第三方密码库内部状态全部零化。
种子/验证码不实现Clone、Serialize、Display，Debug完全脱敏。真实随机种子生成、二维码、TTY仍未交付；恢复码/替换内部Interface见v4补充，不是生产装配。

两个句柄是可信进程内职责隔离，不是OS安全隔离。未来HTTP只可拿普通句柄，不能拿登记句柄、种子、SessionManager装配依赖。
localhost、请求头、布尔值或仅邮箱登录均不能当可信本地登记授权。

## 固定验证码与限流

- 标准HMAC-SHA1、6位ASCII数字、30秒步长、UNIX epoch起算、±1步；SHA1不用于密码哈希或签名。
- 完整比较三个候选；成功计数器全因子严格递增。多个未用同码候选取最大，任一已消费候选同码则整体拒绝。
- 第零步不下溢，2038之后及可表示毫秒上界不截断；格式错误在解析面拒绝，不计算OTP或扣预算。
- 账号与可信来源分别5次受理/900000毫秒，从该桶首个受理时间起固定窗口；成功也不返还或清空预算。
- 网络来源最多64个未到期桶，另有固定account/local桶；不提前驱逐活跃桶，满额失败关闭。
- 来源按地址族与网络字节域分离摘要，IPv4-mapped IPv6归一到IPv4；不是匿名化承诺。
- 先持久预扣双桶，再计算OTP。Cookie、更换来源、重启、失败审计补记不能清空账号限制。
- 使用会话的同一持久时间地板；预期拒绝提交时间/到期/预算，真实存储失败不继续认证。

## 持久状态与发布

v3不改v1/v2已应用DDL，只增设五个固定STRICT表（完整对象数25）：

| 表 | 逻辑预算与用途 |
|---|---|
| `totp_control` | 唯一正i64操作序号，失败可烧号，不复用 |
| `totp_factor` | 至多1行；引用、摘要、最大消费步、最后消费Operation锚点、首次登记双回执与激活标记 |
| `totp_pending` | 全主人至多1笔不可变决定与双阶段回执 |
| `totp_grants` | 每保留会话至多1份最新已确认L3证明；会话删除时级联回收 |
| `totp_rates` | 至多66桶，不存原始来源地址 |

预算受理和OTP消费分为两个Auth事务；消费时重新验会话、种子、时间窗口与全因子高水位。
正确OTP消费与唯一pending同事务落盘，只收紧权限，不授权；然后在Auth事务外确认intent/outcome。
审计outcome只表示允许激活的决定，不声称已登记、已返回或目标动作已执行。
双回执事件/编码/完整性必须一致，intent offset小于outcome；坏ACK在写前或提交前拒绝，不把坏外部回执写成永久损坏状态。
重确认已有登记和待替换旧grant的原事件时，必须拿回精确原回执，不能混合另一审计历史。

最后仅原调用重验相同pending、epoch、身份、活跃会话、CSRF与期限，再原子激活登记/替换该会话proof并清pending。
L3从**消费时刻**起300000毫秒，受会话absolute与该次Access到期封顶；审计等待占用窗口，不能续期。
首次受理到消费也限300000毫秒；消费前时间推进不会暗中缩短成功后的窗口。
补记、重开和另一实例均不能激活。提交成功与客户端实际收到响应不能原子交付，失败可留无响应的已授权会话状态。

审计故障可以烧掉OTP并留下pending，但不授予L3；普通L2吊销不受此等待阻塞。
pending到期、epoch失配或所依赖会话已终态后，仍须双原事件交付才能释放。
会话维护不能回收被pending引用的终态；释放后沿原v2审计终态回收，latest grant随会话级联删除。
已确认旧grant仅在原回执重确认后原子替换，完整历史仍由AuditLedger保存。
最后消费Operation锚点与步数始终保留；失效pending/旧session消失不能使步数回退或重新授权。

## 完整性与保证范围

每次数据库事务检查固定结构、行数、长度、JSON规范编码、引用与回执，再执行逻辑。
Operation≤2048字节，单回执≤8192字节；先SQL有界检查后解析，未知字段/重复或错配序号/seed引用/会话主键拒绝。
消费锚点可引用已经安全回收的会话，但pending和grant必须仍有对应会话；锚点不是授权证明。
迁移单事务内失败回滚，不登记或提权；提交后的时钟/存储检查仍可能使方法返回Err而库已合法v3，修复外部条件后幂等重开。

不抵御管理员一致重写、Auth整库回滚/克隆双写，不证明TOTP抗实时钓鱼；不是硬件因子或完整公网上线验收。
SQLite文件/WAL高水位和AuditLedger历史可增长，逻辑行数有界不代表墙钟耗时固定。
真实CLI、密钥平面部署、恢复码保管、丢机恢复与因子轮换实操、Cookie/Origin/代理接线和用户真实验收仍是生产硬门槛。
没有清库重登、邮箱移除TOTP或默认恢复成功的弱入口。

依据：[RFC6238](https://www.rfc-editor.org/rfc/rfc6238.html)、[RFC4226](https://www.rfc-editor.org/rfc/rfc4226.html)、
[OWASP MFA](https://cheatsheetseries.owasp.org/cheatsheets/Multifactor_Authentication_Cheat_Sheet.html)、
[RustCrypto HMAC](https://docs.rs/hmac/0.12.1/hmac/)。具体通过证据与未执行项见[M2记录](../m2/README.md)。
