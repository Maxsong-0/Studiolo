# 新根恢复：取回副本怎么核、怎么放、怎么恢复、怎么切换

T-204（2026-11-15）做了取回副本的只读核验和逐字节放置；T-212（2026-11-16）把原来的「整次拒绝」换成真实的
恢复：Auth 撤权、主服务启动闸门、黑板三件（Control、AgentStore、Policy）的失效、接手上一次、命令行
`studiolo-restore`、两步最终切换。测试箱（108，x86_64）上的用例用真实 Broker、Auth、人侧账本和真实的六 owner
生产者封存跑通。同日按 Codex 第二意见又改了六处：带凭据的库先放在暂用名下、恢复标识不许复用、被封存的根
本身是恢复出来的也能恢复、每个 owner 先核再记「开始」、接手时按组件认已放好的、控制库只读打开（下文各处标了
出处）。

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
  已核验视图重核已放好的组件（多一个、少一个、改一个字节都拒绝）；放在它里面的组件（接入方账本 `audit`）另算。
- 带凭据的库先放在暂用名下，由它的 owner 做完失效之后用 `publish_held` 改成正式名：同一父目录里原子改名，正式名
  已经在就失败、不覆盖，改完同步父目录。
- 新目录是属主可写的（目录 0700、文件 0600），因为各库接下来要在上面继续写。
- 新根必须已经存在、属于当前账号、0700；和取回副本要在同一个账号下，不要求同一个文件系统。

## 恢复编排（`restore_into_new_root`，在主服务的 `checkpoint` 模块）

输入：已核验的取回副本、新根、恢复标识（1～55 个字母、数字或 `-`）、恢复记录（`RecoveryGrant`）、人确认过的
八项前置检查、主人的会话配置（私有配置里的）、主人的 TOTP 种子、恢复记录的去处（每追加一条先交给它）。

### 名单：哪些 owner 怎么恢复

只看已核验的清单，不碰磁盘：

| owner | 放到新根哪里 | 恢复之后做什么 |
|---|---|---|
| Broker | `vault`（canonical）、`data`（writer-data） | 专用重绑定 `rebind_new_roots`：保留 Vault 身份和配对标识，重建物理绑定，lineage 记下恢复标识和这份数据以前用过的全部恢复标识 |
| Audit | `audit`；有 AgentStore 时接入方账本放在身份库目录里（`agent.held/audit`，改名之后是 `agent/audit`） | 只开已有库，核链尾（见下面第 9 步） |
| Control | `control` | 收回全部租约 |
| Policy | `policy` | 一律全局吊销 |
| AgentStore | 先放 `agent.held`，吊销做完改名 `agent` | 吊销全部类型 token，每个实例在策略库记实例吊销 |
| Bridge（双向同步） | 不放 | 不恢复运行状态，记 `SyncLeftClosed`，同步按引导流程从零重新打开 |
| Auth | 先放 `auth.held`，撤权做完改名 `auth` | 撤权，永远最后做 |

Broker、Auth、Audit 必须在场。缺一个、水位缺项或多出、和名单对不上、同一个 owner 的水位出现两次、Broker 还有
未决写入、人侧账本是空的、Bridge 的持久门禁开着、有 AgentStore 却没有策略库（实例吊销记在策略库里）、有
AgentStore 却没有接入方账本或者反过来，都在写任何东西之前拒绝（`Roster`）。名单里缺席的 owner，新根上不会
出现它的目录；这里不建空库、不补默认 owner。这一步不碰磁盘（`plan_from`），有逐条的单测。

**暂用名**（Codex 第二意见第 2 条）：带凭据的两个库（Auth、身份库）在它的 owner 把旧凭据全部作废之前，按正式
路径谁都打不开，包括不带写者数据根、不经启动闸门的只读模式和 agent-server。启动闸门照旧是写者模式的那一道。

### 顺序

1. 核恢复标识（`RestoreId`）、恢复记录是这个 checkpoint 的而且有写入授权（`Transcript`）、八项前置检查全部确认
   （`ChecksIncomplete`）。
2. 按名单定能不能恢复（`Roster`）。
3. 新根里有上一次放置中途失败的 `<名字>.restoring`（正式名、暂用名都算）：拒绝（`CleanupRequired`），声明接手
   也一样。新根里已经有固定名字（正式名、暂用名都算）、调用方又没有声明接手：拒绝（`AlreadyPlaced`），不覆盖。
   已放好的按组件记，不按名字（Codex 第二意见第 4 条：接入方账本和人侧账本的名字都叫 `audit`）。
4. 「新开 / 接手」和恢复记录对得上：新开要用没有任何放置意图的记录；接手要接在记过这次放置意图的那份上
   （`Transcript`）。
5. 接手、而且 Broker 重绑定还没开始的：已放好的组件逐个 `verify_placed`（`Placement`）。改正式名在 owner 失效
   之后，所以这时带暂用名的只能在暂用名下；正式名和暂用名同时在（有人在新根上另建了正式名）也拒绝。

   第 1～5 步的拒绝都发生在写之前，恢复记录也不多一行（有用例）。

6. 记 `PreRestoreChecksCompleted`、`PhysicalRebindIntent`（载荷是恢复标识和这次要恢复的 owner 名单）。
7. 逐字节核对着放置组件，带凭据的放在暂用名下；接手时已放好的不重写。
8. Broker 重绑定，回执里的 Vault 身份和配对标识必须等于清单里的（`Rebind`）。先按 data 里读到的 lineage 判断
   （`classify`，有单测）：
   - 没有 lineage：这份数据从没恢复过，lineage 没有上一环。
   - 别的 checkpoint 的、已完成的、带恢复标识的：被封存的根本身是恢复出来的，封存把它的 lineage 连同 data 一起
     带了过来（Codex 第二意见第 5 条，以前这里整次拒绝）。它只当上一环：新 lineage 的 `parent_lineage` 是它的
     恢复标识，`earlier_restores` 是它那一串加上它自己的。
   - 同一 checkpoint、同一恢复标识、停在重绑定中途的：接着绑（要声明接手）；已完成的：只认同一次恢复，不再
     重绑（要声明接手）。
   - 别的一律拒绝（`Rebind`）：Vault 身份或配对标识不同；别的 checkpoint 的却没完成或没有恢复标识（第 1 版
     接不成链）；同一 checkpoint 的却是别的恢复标识。

   这次的恢复标识这份数据以前用过（在 `earlier_restores` 里）：Broker 在写任何东西之前拒绝（`RestoreIdUsed`，
   Codex 第二意见第 1 条）。Auth 里留着那次的撤权记录，再用一次启动闸门会把旧记录当成这次的。记
   `PhysicalRebindOutcome`。
9. 核两条账本的链尾（`Audit`）：人侧账本水位那一条必须原样还在，之后只许有这次恢复 Auth 撤权自己记的事件
   （`session.revoked`，范围是 restore、目标是这个恢复标识，最多两条）；接入方账本水位之后一条都不能多
   （恢复从不往里写）。
10. 黑板三件依次失效：Control → Policy → AgentStore（`Owner(owner)`）；名单里有 Bridge 的记 `SyncLeftClosed`。
11. Auth 撤权（`Auth`）。

每个 owner 的失效（第 10、11 步）都一个形状（`run_stage`）：这次恢复里还没开始过的，先核对（按清单里它的
水位重算；Auth 核这个恢复标识在它自己的记录里没用过，用过就 `RestoreIdUsed`），**核过了才记**
`OwnerRestoreIntent`，再做失效、做事后核对，记完成；已经开始过的（接手上一次），它的库已经改过，不再按封存
水位核，只做幂等的失效和事后核对。核对必须在「开始」之前（Codex 第二意见第 3 条）：「开始」一记，接手时就当
这个库已经改过、不再核了，核不过的库会被当成核过的直接改。完成的那条只记一次，载荷是计数，不带任何凭据。

核对不改库的内容（不迁移、不写数据、不写绑定）。控制库连 journal mode 都不动；身份库、策略库、Auth 只有各自
那一个打开入口，打开时会把封存导出时的 DELETE 模式切回它们平时运行的 WAL。这一项不进水位，也不影响接手：
核对都在 Broker 重绑定之后，那之后不再逐字节重核已放好的组件。各 owner：

- **Control**（`ControlLeasesRevoked`）：`ControlDb::open_existing` 只开已有库（不建、不迁移、不切 journal
  mode、不写 Vault 绑定，Codex 第二意见第 6 条），按启动巡检收回全部租约（`reconcile_leases(now, true)`）。事后
  核对：没有 attempt 还拿着租约（已认领、运行中）；在等子任务的转成取消中，工具能力已经收回。过期提交停在待审，
  恢复不替主人丢（2026-11-16 拍板）。
- **Policy**（`PolicyGloballyRevoked`）：一律全局吊销（2026-11-16 用户拍板），只能由主人经 L3 解除，再逐个放行。
  封存截止点时已经是全局吊销的，原记录保留、不再加一条。
- **AgentStore**（`AgentCredentialsInvalidated`）：吊销全部类型（旧类型 token 一律失效），每个已登记实例在策略库
  记实例吊销（不可恢复）。事后核对每个实例在策略库都是吊销状态，然后把 `agent.held` 改名 `agent`。之后由主人
  重新签发类型 token、AI 重新登记。
- **Auth**（`AuthInvalidated`）：照 `studiolo-auth-admin restore` 的顺序，只为恢复打开已有库 → 挂 TOTP（主人
  给的种子）和恢复码因子 → 废止全部旧会话、L3 和恢复码（同一恢复标识幂等）。要求身份版本恰好加一（接手时
  等于当时提交的那次、之后没再变）、两条恢复事件都投进新根的人侧账本。Auth 句柄和因子都关了之后，把
  `auth.held` 改名 `auth`。载荷是新旧身份版本和三个计数。

改名失败算这个 owner 没做完（`Owner(AgentStore)`、`Auth`）；接手时失效照旧幂等地再做一遍，再改名。

Auth 撤权放最后：启动闸门只认「这次恢复的 Auth 撤权做完了」，它做完就说明前面每一步都做完了。写了之后任何
一步失败，都停在「新根起不来」，由人看过再接手。唯一的例外是最后一条 `AuthInvalidated` 交不出去：撤权已经
生效、Auth 已经是正式名，闸门会放行；这时旧凭据已经全部失效，只是恢复记录少这一条。最终切换的「开始」要它，
接手补记（撤权幂等，不再撤一次）。有用例：这一条交不出去时闸门放行、切换开始不了，接手补记之后切换能开始；
撤权做完、还没改名就停下（用改回暂用名模拟）时起不来，接手做完；身份库的两个同样的断点也各有用例。

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
  只放好了一部分的（比如六 owner 的名单只放了 `vault`、`data`、`audit`），接着放剩下的，做完（有用例）。
- 重绑定已经做完的：lineage 必须是同一次恢复，不重绑（有用例：换恢复记录、拿用过的记录去另一个新根，都拒绝）。
- 人侧账本链尾多出的条目只能是这次 Auth 撤权自己记的（有用例：多一条别的事件就拒绝）。
- 某个 owner 的失效开始过的，只做幂等的失效和事后核对；再接手一次，身份版本不变，`AuthInvalidated` 不记第二次
  （有用例）。
- 某个 owner 的核对没过的：记录里没有它的「开始」，再接手照旧先核、照旧拒绝，永远不会跳过一次没过的核对
  （有用例：控制库在核对之前被改过，接手两次都拒绝在 `Owner(Control)`，记录里都没有「开始」，身份库还在
  `agent.held` 下）。

## 启动闸门（`require_restore_revoked`）

主服务打开 Auth 之后、HTTP 就绪检查之前调（`startup/restore_gate.rs`），跟着数据走，不跟着路径走：

- 写者数据根上没有恢复 lineage：原生库，照常启动。
- 有 lineage：要 Auth 里有同一恢复标识、两条恢复事件都已投进账本的撤权记录，否则以 `RestoreRevocationPending`
  拒绝启动。第 1 版 lineage 没有恢复标识，一律当撤权没做；lineage 读不出来也不放行。
- 改配置、换目录名都绕不过：lineage 和撤权记录都在数据里（写者模式下把配置指到 `auth.held`，闸门照样按那份
  Auth 核，撤权没做完照样拒绝；例外见下一条）。Broker 重绑定没完成（lineage 停在 `Rebinding`）由 Broker 打开时
  自己拒绝。
- 恢复标识不会撞上这份数据以前的：Broker 按 lineage 记的那一串在重绑定之前拒绝复用，Auth 撤权开始之前再按
  Auth 自己的记录核一次。后一种只在 Auth 里有不经编排写下的撤权记录时才会碰上（比如演练工具在这份数据上撤过
  权），它拒在重绑定之后：lineage 已经记着这个标识，Auth 里又有同一标识的旧记录，这时把配置指到 `auth.held`，
  闸门会被旧记录骗过，只剩暂用名这一层（用例里断言了这一点）。核对挪不到重绑定之前：重绑定之前已放好的组件
  在接手时要逐字节重核，而打开账本和 Auth 会开写事务、生成 SQLite 侧文件，中途被打断留下的侧文件会让重核过不去。
- 有用例：撤权之前（包括 TOTP 种子不对、Auth 撤权失败）不放行；lineage 换成别的恢复标识、第 1 版没有标识，都不
  放行；同一标识撤权做完才放行。旧会话、旧 L3、旧恢复码在撤权后都不再有效，由 Auth 的
  `restores_real_replaced_factor_preserves_proofs_and_retires_all_old_authority` 用例证明。
- 有用例（从恢复出来的根的封存再恢复）：用祖先 lineage 里的标识，Broker 在重绑定之前拒绝；用 Auth 里记过、
  lineage 里没有的标识，Auth 核对时拒绝。两种都停在「新根起不来」（Auth 还在暂用名下），声明接手也照旧拒绝。
  换一个没用过的标识才做完：lineage 的上一环是祖先的标识，那一串记着它，身份版本加一，闸门放行。Broker 那边另有
  用例：那一串满 256 个时再恢复就拒绝、lineage 一个字节不改；停在重绑定中途的同一次恢复接手时沿用那一串。

## 命令行（`studiolo-restore`）

```text
studiolo-restore run --config <私有配置> --checkpoint-copy <取回的副本> --checkpoint <封存标识>
    --seal-sha256 <64 位小写十六进制> --new-root <新根> --restore-id <恢复标识>
    [--resume] --confirm <检查名>（八项各一次）
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
  恢复标识这份数据以前用过（`RestoreIdUsed`）的例外：接手解决不了，下一步是删掉这次放进新根的东西、换一个没用过
  的恢复标识、不接恢复记录从头来。
- 上一环由数据定，命令行不收（以前的 `--parent-lineage` 现在是未知参数，按用法不对拒绝）。
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
- 不带写者数据根启动（只读模式）时没有 lineage 可查，启动闸门管不到；这条路由暂用名管：Auth 撤权做完之前它在
  `auth.held` 下，按正式路径打不开。**现在生产人侧的启动器（`deploy/human/launch-human.py`）跑的就是只读模式**
  （只带示例库、会话配置和策略库，不带写者数据根），靠的就是这一层。agent-server 是另一个程序，没有这道闸门，
  同样由暂用名管：身份库吊销做完之前在 `agent.held` 下。有人特意把配置指到暂用名，只读模式和 agent-server 都
  绕得过去（写者模式在「只在 Auth 里复用恢复标识」那种情况下也一样，见启动闸门一节）：配置不要指向 `*.held`，
  被拒的新根删掉重来；最终切换之前也不要把 agent-server 指到新根。
- 被封存的根是用第 1 版 lineage 恢复出来的（没有恢复标识，T-212 之前的演练布局）：接不成链，拒绝（`Rebind`）。
  lineage 那一串最多记 256 个之前用过的恢复标识：已经记满 256 个的数据再恢复一次就拒绝。
- 身份库、策略库、Auth 的核对会把封存导出时的 DELETE 模式切回 WAL（见各 owner 的形状那一段）；核不过的新根，
  拿它和取回副本逐字节比时，这几个库的文件头会不一样。
- 命令行判断 SSH 只看环境变量。
- 标准输出写到一半失败（比如磁盘满）时，最后一行可能不完整，读回时整份拒绝；那一步没有做，删掉不完整的最后
  一行再接手。
- 恢复之后所有 AI 要重新登记、主人要逐个放行（策略库全局吊销、类型 token 全部吊销），这是用户接受的代价。
- 水位核到的程度：Broker 核身份和配对标识（不核已提交序号）；两条账本核链尾；Control、AgentStore、Policy
  按封存规则在恢复出来的库上重算整份水位；Auth 不核水位，撤权之前核这个恢复标识没用过，撤权之后核身份版本恰好
  加一。字节层面的完整性由取回副本的整棵重算保证。
