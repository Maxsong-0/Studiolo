# 秘密驻留面契约（T-050 措施，T-059 统一实现，T-065 拆成独立 crate 并覆盖第四个程序）

> 范围：明文秘密在**持有它的进程**内存里活多久、经过哪些副本、哪些路径能把它读出去，
> 以及该进程能关掉哪几条、关不掉哪几条。
> 关不掉的一律写成明确的「不支持」，不静默跳过后宣称已加固。
> 本契约**不**宣称抵御 root、持调试授权的一方，或已经能读本进程内存的一方。

**适用进程（2026-09-17 T-065 起共四个，共用同一份实现）**：

| 可执行程序 | 调用点 | 持有的明文 |
|---|---|---|
| `studiolo-auth-admin` | `main::run()` 第一条语句 | TOTP 种子、验证码 |
| `studiolo-local-server` | `startup::prepare_composed()` 第一条语句，**与 `--factor-socket` 无关** | 会话令牌、CSRF 令牌，因子模式另有 TOTP 种子 |
| `studiolo-maintain` | `maintenance::run()` 第一条语句，早于参数解析 | Broker 读进内存的原 proposal 与维护票据 |
| `studiolo-agent` | `main()` 第一条语句，早于参数解析，**与执行哪个子命令无关** | `admin type-issue` 铸出的 32 字节类型 token、每次请求头里的 bearer |

实现只有一份：独立 crate `crates/process-safety/`（`src/lib.rs` 入口、`src/platform.rs` 分平台、
`src/report.rs` 固定分类）。**它的依赖只有 rustix，零工作区依赖**——这不是洁癖，是可用性前提：
实现原先住在 `studiolo-host-config`，而 host-config 会把 `studiolo-auth` 整条认证栈拉进来，
依赖策略（`crates/studiolo/tests/architecture_support/policy.rs`）不允许
`studiolo-agent-server` 依赖它，于是第四个程序接不上。拆成零依赖的独立 crate 之后，
任何持有秘密的程序都能无条件依赖它。该「零工作区依赖」性质本身由依赖策略钉住。

四处调用点都由可执行守卫钉住，任何 crate 自持一份部分收紧即为失败
（`crates/auth-admin/tests/process_discipline.rs`、`crates/local-server/tests/composition/residency.rs`、
`crates/agent-server/tests/process_residency.rs`）。

**T-059 修掉的真实缺口**：收紧原先埋在 `startup/factor_input.rs` 的 `Inputs::load` 里，
只有 `options.factor` 为 `Some` 才走得到，而且只有禁 core 一项；非因子模式的
`studiolo-local-server` 因此**一项都没有**，其进程内存却仍持有 `SessionToken` / `CsrfToken`
明文。`studiolo-maintain` 同样内联了一份只禁 core 的实现。两处都已删除，改为调用共享实现。

**T-065 修掉的真实缺口**：`studiolo-agent`（`crates/agent-server`，**线上 4400 端口正在跑的那个
进程**）是第四个持有明文秘密的程序，却因为接不上 host-config 而**一项收紧都没有**——
它内存里的类型 token 是 MCP 供给面的全部身份。搬 crate 之后已接线，失败语义与另外三处一致：
本平台声明支持的能力没生效即**拒绝启动**。

## 1. 明文秘密清单（横向扫描结果）

进程内一切以明文形式存在的秘密都必须落在零化容器里。下表是全量清单，
新增秘密必须同时进表并选定容器。

| 秘密 | 类型 / 位置 | 容器 | 存活范围 |
|---|---|---|---|
| TOTP 种子（20 字节） | `TotpSeed`：`crates/auth/src/session/totp/secrets.rs` | `Zeroizing<[u8; 20]>` | 从解码到 `Totp` 实例释放 |
| 种子的 hex 传入副本 | `crates/auth-admin/src/environment.rs` | `Zeroizing<Vec<u8>>` | 单次读取，解码后立即释放 |
| TOTP 验证码 | `TotpCode`：`crates/auth/src/session/totp/secrets.rs` | `Zeroizing<String>` | 单次校验 |
| 终端逐字节输入的验证码 | `Input::digits`：`crates/auth-admin/src/terminal.rs` | `Zeroizing<[u8; 6]>` | 单次输入 |
| 会话令牌 / CSRF 令牌 | `SessionToken` / `CsrfToken`：`crates/auth/src/session/secrets.rs` | `Zeroizing<String>` | 单次签发或校验 |
| 恢复码 | `RecoveryCode`：`crates/auth/src/session/recovery/secrets.rs` | `Zeroizing<String>` | 单次换发或核对 |
| HKDF 输出、hex 编码、HOTP 截断等中间态 | `session/secrets.rs`、`recovery/secrets.rs`、`totp/crypto.rs` | `Zeroizing<..>`，另有一处显式 `zeroize()` | 单个函数栈帧 |

**不属于秘密**：JWKS 验签公钥（`crates/auth/src/access/keys.rs`）、各类 SHA-256 指纹与摘要、
会话 ID、审计记录。它们不进零化容器，也不受本契约约束。

**明文离开进程的唯一出口**是 `expose_secret()`，调用点集中在受信传输层
（`crates/local-server/src/auth_http/` 的 cookie 与 CSRF 响应）。新增调用点等同于新增驻留面。

### 1.1 已知的副本收敛

- `TotpSeed::from_hex` 不再把解码结果按值复制出零化容器（旧写法 `Self::new(reference, *bytes)`
  会在本栈帧与 `new` 的参数槽各留一份不受零化覆盖的解码结果）；两条路径现在共用内部
  `bind(reference, Zeroizing<[u8; 20]>)`。
- `TotpSeed::new` 的按值参数是调用方给的拷贝，封装后本函数这一份立即就地清零；
  **调用方自己保留的副本不受影响**，由调用方负责——这条有回归测试钉住。
- 编译器仍可能在寄存器或溢出槽里留下无名副本。零化只对**具名缓冲**成立，本文不作更强声称。

## 2. 四项措施与固定分类

上表的每个进程都在**读取任何秘密之前**调用
`studiolo_process_safety::harden()`，逐项收紧并返回封闭分类。
分类只有两种成功取值：「已生效并回读确认」与「本平台不提供」。
本平台声明支持却没能生效时**整体拒绝并且不读取秘密**，不降级成「没成功也照跑」。

| 措施 | 做什么 | 回读确认方式 |
|---|---|---|
| 核心转储限额 | `RLIMIT_CORE` 软硬限额降为 0 | `getrlimit` 回读两个值均为 0 |
| 进程可检视属性 | Linux `PR_SET_DUMPABLE(0)` | `PR_GET_DUMPABLE` 回读为不可转储 |
| 秘密页驻留 | Linux `mlockall(CURRENT\|FUTURE\|ONFAULT)` | `/proc/self/status` 的 `VmLck` 大于 0 |
| 环境槽位 | 不清除系统环境副本（见 §4） | 进程自身此刻是否仍读得到该槽位 |

顺序固定：先挡崩溃取证，再挡在线读取，最后挡换出，末项登记环境槽位结论。
**与命令、模式无关**：登记工具的迁移与补记、服务器的**非因子模式**、维护工具的 `--help`、
`studiolo-agent` 的 `--help` 与注定失败的管理子命令，这些不读种子的路径同样收紧——
崩溃转储会把配置路径、会话状态与令牌一起落盘。
加固发生在参数解析、配置加载与种子读取之前，这一时序由测试钉住。

分类必须被**记录**：四个入口都把逐项结果写到标准错误（只含措施名与固定分类，
永不含种子、验证码或任何环境值），格式固定为 `秘密驻留面：<措施>=<分类>；…`。
只在内部返回而不记录，等于把「不支持」藏起来。
一次装配**收紧一次、记录一次**：记录条数由真实子进程测试逐条数出来，多一条说明又有人
加了一份私有收紧，少一条说明分类被藏了起来。

## 3. 平台差异（显式建模，不用 `#[cfg]` 抹平）

平台差异建成**两套互斥的取值**：只有 Linux 能构造 `Applied::NotDumpable`，
只有非 Linux 能构造 `Unsupported::NoDumpableSwitch`，编译器因此不允许把缺失能力说成已生效。

| 措施 | Linux | macOS |
|---|---|---|
| 核心转储限额 | 已生效并回读 | 已生效并回读 |
| 进程可检视属性 | 已生效并回读 | **不支持**：没有可转储属性开关 |
| 秘密页驻留 | 已生效（缺页锁定，旧内核退回预先锁定） | **不支持**：无可用 `mlockall`；按页 `mlock` 需要 unsafe |
| 环境槽位 | 不支持（未清除，但外部读取已被可检视属性挡住） | 不支持（未清除，且同用户仍可读出） |

macOS 读他进程内存要经 `task_for_pid`，默认只对 root 或持调试授权者开放——
那是**系统策略**，不是本进程设的开关，因此不记为等价保护。
macOS 的环境块另有 `KERN_PROCARGS2` 一条同用户可读路径，本进程无法关闭。

环境槽位这一行的结论**由本次实测的可检视属性推出**，不按平台常量假定：
措施退化或被平台拒绝时，结论自动退到「同用户仍可读」。

## 4. 为什么没有 `remove_var`

T-026 复审第⑥组原文要求 `remove_var`。逐条核实后不采用，理由是两条独立的硬事实：

1. **调不了**：`std::env::remove_var` 在 edition 2024 是 `unsafe fn`，
   而工作区 lints 是 `unsafe_code = "forbid"`（`forbid` 在 crate 内无法被 `allow` 覆盖）。
2. **就算调了也达不到目的**：它只摘掉 libc 的 `environ` 指针表，
   不改写内核在初始栈上保留的那份字符串；Linux 的 `/proc/<pid>/environ`
   与 macOS 的 `KERN_PROCARGS2` 读的都是内核那一份，仍会原样读出。

`remove_var` 唯一真正能挡的是「进程内稍后再次读到该槽位」与「子进程继承」。
这两条已由结构约束替代，并由测试钉住：

- 整个登记程序只有**两个**读取进程环境的地方——`environment.rs` 的种子槽位与
  `main.rs` 的命令行参数；多一个读取点就多一份已零化秘密的新副本。
- 登记程序**不派生任何子进程**（`Command::new` 在全工作区架构守卫里就被禁），
  因此不存在「清除必须早于 fork/exec」的窗口。

真正挡住外部读取的是**可检视属性**（Linux），不是清除本身。macOS 上这条挡不住，
如实记为「同用户仍可读出」。

需要更强保证时，正确方向是**不再经环境变量传入种子**（改用一次性 fd 或受控注入），
那是产品边界变更，属 `host-config` 的 `seed_ref` 契约，不在本次范围。

## 5. 运维前提：`RLIMIT_MEMLOCK`（T-059 扩及长驻服务，T-065 再扩及 `studiolo-agent`）

Linux 上 `mlockall` 受 `RLIMIT_MEMLOCK` 约束。上限不足时 `harden()` 返回
「可锁定内存上限不足，请提高 RLIMIT_MEMLOCK 后重试；未读取秘密」并终止——
这是**有意的失败关闭**：锁不住就不能承诺秘密页不被换出。

因此运行这四个程序的环境都必须给足可锁定内存；交互 shell 里用 `ulimit -l` 查看。
本项默认值在各发行版与容器运行时之间差异很大（systemd 常见默认 `DefaultLimitMEMLOCK=8M`），
**不要假定「够用」**。

- 短命 CLI（`studiolo-auth-admin`、`studiolo-maintain`、`studiolo-agent admin …`）常驻集小，
  通常几十 MB 足够。
- **长驻服务是另一档**：`studiolo-local-server` 与 `studiolo-agent serve` 的常驻集远大于 CLI，且
  `mlockall(MCL_FUTURE)` 对其后每一次新映射同样有效。运行它们的 systemd 单元必须显式写
  `LimitMEMLOCK=infinity`（或一个覆盖其常驻集上限的值），否则进程会按设计**拒绝启动**，
  而不是降级成「没锁上也照跑」。六处都已补上（2026-09-17）：
  `deploy/preview/studiolo-preview.service.in`、`deploy/preview/studiolo-access.service.in`
  （T-059）；`deploy/agent/studiolo-agent-system.service.in`、
  `deploy/agent/studiolo-agent.service.in`，以及 `deploy/agent/install-t036-system.sh` 与
  `deploy/agent/upgrade-read-supply.sh.in` 里 `systemd-run` 起的 4401 阶段验收临时实例
  （`5ff4985`，PR #7——它先于本轮加固落地，正是为了让加固接线时不必再动部署单元；
  阶段实例同样会被 memlock 卡住，漏了它阶段验收会先挂）。
  **升级线上 `studiolo-agent` 时必须先让新单元生效，否则服务起不来**——
  这不是重启顺序问题，是部署前置。
- CI 同理，且**每个 `run` 步骤是独立 shell**，上一步抬高的上限不会传到下一步：
  `.github/workflows/ci.yml` 在「Workspace 测试」「Python 副本发布与真实 MCP 管线」
  「本地登记工具生产特征构建」三个步骤里各抬一次
  （`sudo prlimit --pid $$ --memlock=unlimited:unlimited`）；
  `scripts/test-linux.sh` 的 docker 用 `--ulimit memlock=-1` 一次覆盖整个容器。
- 发布包验收 `scripts/run-agent-tests.sh` 在服务器上以普通用户跑，抬不了自己的硬上限，
  因此它**先检查 `ulimit -l` 再开跑**：低于 64 MiB 直接以明确原因退出，
  免得整片红被误读成代码坏了。

## 6. 测试证据与未覆盖项

测试分五处，刻意不满足于「调用过某个系统调用」：

- `crates/process-safety/tests/support/process_safety_tests.rs`：实现本身的固定分类、
  内核回读、真实子进程外部检视与幂等性。
- `crates/local-server/tests/composition/residency.rs`：**因子与非因子两条路径**各起一个
  真实子进程跑完整装配并回读内核，外加「记录恰好一次」与源码时序/唯一实现守卫。
- `crates/local-server/tests/maintenance/terminal_process.rs`：真实 `studiolo-maintain`
  二进制在真实 PTY 下，收紧与记录排在任何参数处理之前。
- `crates/auth-admin/tests/process_discipline.rs`：登记程序的唯一环境读取点、
  加固最先发生、不派生子进程、自有缓冲零化。
- `crates/agent-server/tests/process_residency.rs`：**实际 `studiolo-agent` 二进制**——
  `--help` 与一条注定失败的管理命令各跑一次，证明收紧与记录挂在入口而不是挂在某条命令上；
  Linux 上另起一个真实 `serve` 进程，由**父进程**从 `/proc/<pid>/` 回读它的锁定页数、
  核心转储限额与环境块可读性。

共有的取证手法：

- **真实子进程**跑完整加固，把内核回读结果交回父进程：核心转储限额、
  可转储属性（`PR_GET_DUMPABLE`）、锁定页数（`VmLck`）。
- **另一个真实进程**在子进程活着时读 `/proc/<pid>/environ`：
  没有 `CAP_SYS_PTRACE` 时必须得到 `PermissionDenied`；持有它时按设计这条保护不成立，
  测试断言「确实读得到」，不做空跑假绿——本模块从不宣称挡得住它。
  判据是 capability 而**不是** uid：容器默认丢掉 `CAP_SYS_PTRACE`，容器内的 root 同样读不到，
  按 uid 判断会给出错误结论（2026-09-17 实测撞到过）。`/proc/<pid>` 的属主也不是可移植判据，
  只作排障读数，不据此判定成败。
- **环境槽位分类反证**：分类说「未清除」，进程此刻就必须真读得到该槽位；
  将来真的实现了清除，分类与读数必须同时翻转。
- **零化**：定长与变长缓冲各测一次就地清零，另测「释放确实触发零化」这一环。
- **源码守卫**：环境读取点唯一、加固与记录先于一切读取、全程不派生子进程，
  并且**全工作区只有一份实现**——`studiolo-auth-admin` / `studiolo-local-server` /
  `studiolo-agent-server` 三个 crate 的 `src/` 里再出现 `setrlimit` / `getrlimit` /
  `Resource::Core` / `set_dumpable_behavior` / `mlockall` / `remove_var` 即为失败。
- **依赖策略守卫**：`studiolo-process-safety` 的允许工作区依赖是空集
  （`crates/studiolo/tests/architecture_support/policy.rs`）。它一旦长出任何工作区依赖，
  「谁都能无条件依赖它」这条性质就没了，加固又会漏掉某个程序——这正是 T-065 之前发生的事。

**未覆盖 / 已知限制**（不得当成已验证）：

- 释放**之后**的内存内容在安全 Rust 里读不到，本文不声称「释放后已清零」，
  只声称「具名缓冲在释放前就地清零，且释放确实触发零化」。
- 编译器可能在寄存器或溢出槽里留下无名副本，零化覆盖不到。
- **T-065 已在 Linux 实跑并留下证据**（Docker `rust:1.97.1-bookworm`，aarch64，
  `--ulimit memlock=-1`，2026-09-17）：`cargo test --workspace` 179 个测试目标全过，其中
  `studiolo-process-safety` 7 项（含真实子进程内核回读与外部检视）、
  `agent-server/process_residency` 5 项（含父进程回读真实 `serve` 的 `/proc/<pid>/`）；
  实际 `studiolo-agent admin init` 在 Linux 上记录的是「三项已生效＋环境槽位不支持」。
  T-050 与 T-059 当初只在 macOS 上开发、Linux 侧未实跑的那条登记，至此由本轮补上。
- macOS 上**没有**外部证据可取：`studiolo-agent` 的三项「不支持」只由被记录的固定分类核对，
  本文不声称在 macOS 上验证过等价保护——那正是「不支持」这个分类的含义。
- 换出已发生**之前**写入交换区的页、以及休眠镜像，不在本契约范围内。
