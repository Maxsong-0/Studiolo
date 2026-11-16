# 新根恢复：取回副本怎么核、怎么放、怎么恢复、怎么切换

T-204（2026-11-15）做了取回副本的只读核验和逐字节放置；T-212（2026-11-16）把原来的「整次拒绝」换成真实的
恢复：Auth 撤权、主服务启动闸门、黑板三件（Control、AgentStore、Policy）的失效、接手上一次、命令行
`studiolo-restore`、两步最终切换。测试箱（108，x86_64）上的用例用真实 Broker、Auth、人侧账本和真实的六 owner
生产者封存跑通。

**没有验证的**：真实 restic 取回的目录（用例里是把真实封存整棵复制一份来模拟）、真机上从取回到切换的一整轮、
四小时 RTO。这一轮要主人本人在 Mac 上演练（T-212 第 5 个里程碑）。本文件不是「生产上能做新根恢复」的证明。

相关契约：[恢复契约](../m2/checkpoint-recovery-contract.md)「新目录恢复」、[暂存与封存](checkpoint-staging.md)、
[备份生产者](backup-producer.md)、[Broker 封存与新根恢复](broker-checkpoint.md)、[Auth 管理工具](auth-admin.md)
（演练用的 `studiolo-auth-admin restore` 照旧只认演练布局，不放宽）。

## 取回副本（`RestoredCheckpoint`，在 `studiolo-backup`）

从远端取回的封存目录放在哪都行，过了保留期也能核。它和上传用的 `SealedCheckpoint` 是两个类型：追加执行器
只收后者，取回的副本换不成上传能力（把副本放回同名目录再按上传来源重开，仍然被拒，有用例）。

`RestoredCheckpoint::open(目录, 引用, 截止时间)` 的条件：

- **引用（checkpoint 标识加 `seal.json` 的摘要）必须来自可信记录**：人侧账本的准入事件，或远端最终清单。
  副本自己报的摘要不算数。
- 目录属于当前账号、整棵只读（目录 0500、文件 0400）、没有 ACL。取回之后要先把权限收成这样；
  还能被写的目录直接拒绝。
- 顶层只有 `data`、`inclusion.json`、`manifest.json`、`seal.json` 四个名字；`seal.json` 的摘要等于引用，
  清单能由票据原样重建，`data` 整棵逐字节重算后等于封存时登记的视图（多一个、少一个、改一个字节都拒绝）。

和重开原封存相比只少两条：不要求目录还在原来的路径上，不要求还在保留期内。

## 放置（`RestoredCheckpoint::extract`、`verify_placed`）

把 `data/<组件>` 复制到新根下面的固定位置，是封存内容离开取证副本的唯一出口：

- 写的每个文件都对着已核验视图里的摘要和长度，不是「磁盘上现在是什么就抄什么」；核验之后副本又被动过，
  会在写的过程中发现并失败。
- 先写进 `<名字>.restoring`，全部落盘后才原子改名。中途失败留下的 `.restoring` **不自动清**，下次直接
  拒绝（`CleanupRequired`），由人看过再删。
- 目标名字已经存在：什么都不写，原样告诉调用方（`AlreadyPresent`）。接手时由 `verify_placed` 按同一份
  已核验视图重核已放好的组件（多一个、少一个、改一个字节都拒绝）；放在它里面的组件（`agent/audit`）另算。
- 新目录是属主可写的（目录 0700、文件 0600），因为各库接下来要在上面继续写。
- 新根必须已经存在、属于当前账号、0700；和取回副本要在同一个账号下，不要求同一个文件系统。

## 恢复编排（`restore_into_new_root`，在主服务的 `checkpoint` 模块）

输入：已核验的取回副本、新根、恢复标识（1～55 个字母、数字或 `-`）、恢复记录（`RecoveryGrant`）、人确认过的
八项前置检查、主人的会话配置（私有配置里的）、主人的 TOTP 种子、恢复记录的去处（每追加一条先交给它）。

### 名单：哪些 owner 怎么恢复

只看已核验的清单，不碰磁盘：

| owner | 放到新根哪里 | 恢复之后做什么 |
|---|---|---|
| Broker | `vault`（canonical）、`data`（writer-data） | 专用重绑定 `rebind_new_roots`：保留 Vault 身份和配对标识，重建物理绑定，lineage 记下恢复标识 |
| Audit | `audit`；有 AgentStore 时接入方账本放 `agent/audit` | 只开已有库，核链尾（见下面第 9 步） |
| Control | `control` | 收回全部租约 |
| Policy | `policy` | 一律全局吊销 |
| AgentStore | `agent` | 吊销全部类型 token，每个实例在策略库记实例吊销 |
| Bridge（双向同步） | 不放 | 不恢复运行状态，记 `SyncLeftClosed`，同步按引导流程从零重新打开 |
| Auth | `auth` | 撤权，永远最后做 |

Broker、Auth、Audit 必须在场。缺一个、水位缺项或和名单对不上、Broker 还有未决写入、Bridge 的持久门禁开着、
有 AgentStore 却没有策略库（实例吊销记在策略库里），都在写任何东西之前拒绝（`Roster`）。名单里缺席的
owner，新根上不会出现它的目录；这里不建空库、不补默认 owner。

### 顺序

1. 核恢复标识（`RestoreId`）、恢复记录是这个 checkpoint 的而且有写入授权（`Transcript`）、八项前置检查全部确认
   （`ChecksIncomplete`）。
2. 按名单定能不能恢复（`Roster`）。
3. 新根里有上一次放置中途失败的 `<名字>.restoring`：拒绝（`CleanupRequired`），声明接手也一样。新根里已经有
   固定名字、调用方又没有声明接手：拒绝（`AlreadyPlaced`），不覆盖。
4. 「新开 / 接手」和恢复记录对得上：新开要用没有任何放置意图的记录；接手要接在记过这次放置意图的那份上
   （`Transcript`）。
5. 接手、而且 Broker 重绑定还没开始的：已放好的组件逐个 `verify_placed`（`Placement`）。

   第 1～5 步的拒绝都发生在写之前，恢复记录也不多一行（有用例）。

6. 记 `PreRestoreChecksCompleted`、`PhysicalRebindIntent`（载荷是恢复标识和这次要恢复的 owner 名单）。
7. 逐字节核对着放置组件；接手时已放好的不重写。
8. Broker 重绑定，回执里的 Vault 身份和配对标识必须等于清单里的（`Rebind`）；接手时 lineage 已经完成的，只认
   同一次恢复（同一 checkpoint、恢复标识、Vault 身份、配对标识），不再重绑。记 `PhysicalRebindOutcome`。
9. 核两条账本的链尾（`Audit`）：人侧账本水位那一条必须原样还在，之后只许有这次恢复 Auth 撤权自己记的事件
   （`session.revoked`，范围是 restore、目标是这个恢复标识，最多两条）；接入方账本水位之后一条都不能多
   （恢复从不往里写）。
10. 黑板三件依次失效：Control → Policy → AgentStore（`Owner(owner)`）；名单里有 Bridge 的记 `SyncLeftClosed`。
11. Auth 撤权（`Auth`）。

每个 owner 的失效（第 10、11 步）都一个形状：这次恢复里还没开始过的，先记 `OwnerRestoreIntent`，按清单里它的
水位重算核对，再做失效、做事后核对，记完成；已经开始过的（接手上一次），它的库已经改过，不再按封存水位核，
只做幂等的失效和事后核对。完成的那条只记一次，载荷是计数，不带任何凭据：

- **Control**（`ControlLeasesRevoked`）：`ControlDb::open_existing` 只开已有库（不建、不迁移），按启动巡检收回
  全部租约（`reconcile_leases(now, true)`）。事后核对：没有 attempt 还拿着租约（已认领、运行中）；在等子任务的
  转成取消中，工具能力已经收回。过期提交停在待审，恢复不替主人丢（2026-11-16 拍板）。
- **Policy**（`PolicyGloballyRevoked`）：一律全局吊销（2026-11-16 用户拍板），只能由主人经 L3 解除，再逐个放行。
  封存截止点时已经是全局吊销的，原记录保留、不再加一条。
- **AgentStore**（`AgentCredentialsInvalidated`）：吊销全部类型（旧类型 token 一律失效），每个已登记实例在策略库
  记实例吊销（不可恢复）。事后核对每个实例在策略库都是吊销状态。之后由主人重新签发类型 token、AI 重新登记。
- **Auth**（`AuthInvalidated`）：照 `studiolo-auth-admin restore` 的顺序，只为恢复打开已有库 → 挂 TOTP（主人
  给的种子）和恢复码因子 → 废止全部旧会话、L3 和恢复码（同一恢复标识幂等）。要求身份版本恰好加一（接手时
  等于当时提交的那次、之后没再变）、两条恢复事件都投进新根的人侧账本。载荷是新旧身份版本和三个计数。

Auth 撤权放最后：启动闸门只认「这次恢复的 Auth 撤权做完了」，它做完就说明前面每一步都做完了。写了之后任何
一步失败，都停在「新根起不来」，由人看过再接手。

### 恢复记录先交出去，再做那一步

每追加一条恢复记录，先交给调用方的去处（命令行写到标准输出并刷新），交出去了才做那一步。交不出去就停，
那一条也从内存里撤掉：人手里的记录和内存里的一样，人手里没有的步骤一步都没做（有用例：Auth 撤权的「开始」
交不出去时停下，拿人手里的文本接手做完）。

### 回执

`RestoreReport`：lineage、这次有没有沿用已放好的组件、恢复了哪些 owner、同步是不是留在关闭、Auth 撤权的结果
（新身份版本和三个计数）。**它不等于服务可以上线**，还差人核对之后的最终切换。

## 接手上一次

写了之后中途失败（或被打断），由人看过原因、清掉 `.restoring` 半成品，然后用同一个恢复标识、接在上一次写出的
那份恢复记录上声明接手：

- Broker 重绑定还没开始的：已放好的组件逐个按已核验视图重核，改一个字节、多一个文件都在写之前拒绝（有用例）。
- 重绑定已经做完的：lineage 必须是同一次恢复，不重绑（有用例：换恢复记录、拿用过的记录去另一个新根，都拒绝）。
- 人侧账本链尾多出的条目只能是这次 Auth 撤权自己记的（有用例：多一条别的事件就拒绝）。
- 某个 owner 的失效开始过的，只做幂等的失效和事后核对；再接手一次，身份版本不变，`AuthInvalidated` 不记第二次
  （有用例）。

## 启动闸门（`require_restore_revoked`）

主服务打开 Auth 之后、HTTP 就绪检查之前调（`startup/restore_gate.rs`），跟着数据走，不跟着路径走：

- 写者数据根上没有恢复 lineage：原生库，照常启动。
- 有 lineage：要 Auth 里有同一恢复标识、两条恢复事件都已投进账本的撤权记录，否则以 `RestoreRevocationPending`
  拒绝启动。第 1 版 lineage 没有恢复标识，一律当撤权没做；lineage 读不出来也不放行。
- 改配置、换目录名都绕不过：lineage 和撤权记录都在数据里。Broker 重绑定没完成（lineage 停在 `Rebinding`）由
  Broker 打开时自己拒绝。
- 有用例：撤权之前（包括 TOTP 种子不对、Auth 撤权失败）不放行；lineage 换成别的恢复标识、第 1 版没有标识，都不
  放行；同一标识撤权做完才放行。旧会话、旧 L3、旧恢复码在撤权后都不再有效，由 Auth 的
  `restores_real_replaced_factor_preserves_proofs_and_retires_all_old_authority` 用例证明。

## 命令行（`studiolo-restore`）

```text
studiolo-restore run --config <私有配置> --checkpoint-copy <取回的副本> --checkpoint <封存标识>
    --seal-sha256 <64 位小写十六进制> --new-root <新根> --restore-id <恢复标识>
    [--parent-lineage <上一次的恢复标识>] [--resume] --confirm <检查名>（八项各一次）
studiolo-restore cutover begin|finish --config <私有配置> --new-root <新根> --restore-id <恢复标识>
```

八项检查名：`sync-adapter-disabled`、`execution-adapter-disabled`、`content-verified`、`version-verified`、
`permissions-verified`、`hashes-verified`、`audit-continuity-verified`、`space-verified`。少一项就在读任何输入之前拒绝，
并列出没确认的。

- **只认本机终端上的人**：标准错误要连着终端，标准输入要从管道或文件接（在终端里敲种子会回显），而且不在 SSH
  会话里（只看 `SSH_CONNECTION`、`SSH_CLIENT`、`SSH_TTY` 在不在：防误操作，不防有意绕开的人）。不满足的什么都
  不读、不写就拒绝。之后先收紧秘密驻留面（和主服务同一份实现），再读种子。
- **标准输入**：`run` 第一行是主人的 TOTP 种子（恰好 40 位小写十六进制加一个换行；就是主服务用的那个：生产机
  凭据目录里主服务的种子槽，新机器上从主人密码管理器取），不进参数、不进恢复记录。新开时只有这一行；接手
  （`--resume`）时种子之后接上一次的完整恢复记录。`cutover` 只接恢复记录。
- **标准输出**：完整的恢复记录，第一行是头（checkpoint 标识），之后每条一行，都是 JSON。已有的先整份写出，新的
  每追加一条就写出并刷新，然后才做那一步。每次运行写到一个**新文件**，接手和切换时拿最新那一份；不要把输出
  写回输入的那个文件（shell 会先把它清空）。
- **标准错误**：中文的进度、结论和下一步。拒绝时按恢复记录里有没有这次的放置意图给下一步：有，就是「新根照旧
  起不来，接这次写出的恢复记录加 `--resume` 接手」；没有，就是「这次没往新根写任何东西，改正之后再来」。
- **退出码**：0 做完；1 拒绝或没做完；2 用法不对（在终端检查之前）。

一轮演练的样子（尖括号里换成实际值；取种子的命令要把种子打成一行到标准输出）：

```bash
studiolo-restore run --config <私有配置> --checkpoint-copy <取回的副本> --checkpoint <封存标识> --seal-sha256 <封存摘要> --new-root <新根> --restore-id <恢复标识> --confirm sync-adapter-disabled --confirm execution-adapter-disabled --confirm content-verified --confirm version-verified --confirm permissions-verified --confirm hashes-verified --confirm audit-continuity-verified --confirm space-verified < <(<取种子的命令>) > restore-1.jsonl
```

中途没做完、看过原因之后接手（参数和上面一样，多一个 `--resume`）：

```bash
{ <取种子的命令>; cat restore-1.jsonl; } | studiolo-restore run <同上的参数> --resume > restore-2.jsonl
```

做完、人核对过新根内容之后，开始最终切换：

```bash
studiolo-restore cutover begin --config <私有配置> --new-root <新根> --restore-id <恢复标识> < restore-2.jsonl > restore-3.jsonl
```

## 最终切换（`begin_cutover`、`finish_cutover`）

由人在核对之后分两步执行；这里不改配置、不起停服务，只核前提，再把这一步记进恢复记录。

1. **开始**：这次恢复的 Auth 撤权已经记在恢复记录里、还没开始过切换；新根的 lineage 是这次恢复的（同一恢复标识
   和 checkpoint、已完成）；启动闸门放行。记 `FinalCutoverIntent`。
2. 人把生产配置指到新根、起服务、主人重新登录并核对内容，然后停掉服务。
3. **完成**：记过开始、还没完成过；主人在新根上重新登录过（撤权之后的身份版本里有激活过的会话）；恢复记录里的
   身份版本等于 Auth 里这次撤权提交的；名单里有 Control 的租约收回过；`AuthRestoreInvalidation::check` 的失效事实
   全部成立（名单里有身份库、策略库的，它们的失效都记过）。记 `FinalCutoverOutcome`。这一步按离线恢复的方式
   打开 Auth，所以新根上的服务要先停。

拒绝类别：`Transcript`、`Sequence`（次序不对，各只做一次）、`NewRoot`（新根对不上、打不开、闸门不放行）、
`NotReauthenticated`、`Invalidation`。不打算切换的（回到旧根）不用这里：旧根没动过，新根由人删。双向同步在切换
之后照旧关着，按 T-132 的引导流程从零重新打开（`BidirectionalSyncGate`）。

## 恢复包（一次性恢复身份）

新开时由命令行从核验过的取回副本当场生成：checkpoint 标识、封存摘要、清单里的秘密引用（只有名字），一次性
身份十分钟内有效，生成后立即在本机终端兑换，恢复记录的第一条是 `KitOpened`。恢复包里没有秘密，所以不单独
保存。恢复真正要的秘密都在主人手里：取回封存用的仓库口令、TOTP 种子；可信引用（封存标识和摘要）来自远端最终
清单或人侧账本的准入事件。

这是对 T-212 设计 4.6「恢复包放进单独的 restic 仓库加密存取」的偏离（2026-11-16，Claude 定，用户可以推翻）：
「每次批准备份配置后存一份恢复包」那时还没有 checkpoint，绑不上；主服务不许起子进程，备份 worker 的 restic
调用绑在它自己的系统凭据上；新加一套加密库又是设计明确不要的。代价是少了一道「恢复包仓库口令」，剩下的门槛
是取回仓库口令、TOTP 种子、本机终端和本机账号。

## 已知限制

- 真实 restic 取回、真机上的一整轮、四小时 RTO：都没有验证，要主人本人演练。取回这一步（restic restore 到
  一个目录、收权限）是人按运行手册做的，没有代码。
- 不带写者数据根启动（只读模式）时没有 lineage 可查，启动闸门管不到；恢复出来的新根必须按生产配置带写者数据根
  启动。agent-server 是另一个程序，没有这道闸门：最终切换之前不要把它指到新根。
- 命令行判断 SSH 只看环境变量。
- 标准输出写到一半失败（比如磁盘满）时，最后一行可能不完整，读回时整份拒绝；那一步没有做，删掉不完整的最后
  一行再接手。
- 恢复之后所有 AI 要重新登记、主人要逐个放行（策略库全局吊销、类型 token 全部吊销），这是用户接受的代价。
- 水位核到的程度：Broker 核身份和配对标识（不核已提交序号）；两条账本核链尾；Control、AgentStore、Policy
  按封存规则在恢复出来的库上重算整份水位；Auth 不核水位，撤权之后核身份版本恰好加一。字节层面的完整性由取回
  副本的整棵重算保证。
