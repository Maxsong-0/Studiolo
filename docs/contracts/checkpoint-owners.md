# 七 owner 实际协调窗口

`CheckpointOwners` 从正在服务的 CanonicalWorkbench、SessionManager、BoardPlane、SyncPlane、PolicyPlane 及实际身份/审计成对能力取得固定源。它不打开任意数据库、不初始化缺失组件、不把已装配组件冒充 absent；黑板存在而身份源缺席时直接拒绝。

作业先通过独立排队流程持久 `backup.snapshot` Intent。Worker 与协调器共用 `backup_intent` 唯一映射；排队方核对返回记录完整性和完整事件，协调器最终在实际冻结的人侧账本中查到同一原事件才继续。单纯传入作业 ID 不能绕过落账。

先取得接入方完整操作独占许可，再取得人侧许可，然后按 Broker → Control → Auth → Bridge → AgentStore → Policy → Audit 冻结。Audit 阶段包含接入方、人侧两个独立账本。冻结窗口从开始到放开使用同一单调截止时间（冻结预算）；Policy 后续导出或重试不能刷新该时间。任一失败按逆序释放：先关闭账本及上游存储屏障，最后恢复人侧、接入方准入。

生产封存走 `freeze_timed` → `release_timed` → `ReleasedOwners::seal_timed`，每走完一步记进 `StepTimes`（BUG-050，2026-11-23），
失败时已走过的照样留着，生产者照它写窗口那一行和放开之后那一行（步名见 `backup-producer.md`）；不计时的 `freeze`／`release`／`seal` 只给用例用。

冻结窗口只定格（BUG-045，2026-11-22）：各 SQLite 库在冻结时做完 WAL checkpoint，再在一条只读连接上开读事务把库定在这一刻；写者两棵树逐个打开文件留句柄、记身份，不读内容。窗口里只做要读活对象、又有固定上限的几步（下一段），然后 `FrozenOwners::release` 按冻结的逆序放开：两本账本、Policy、AgentStore 的写锁，Bridge（整份已导出），Auth、Control 的写锁，写者还回工作台，最后恢复人侧、接入方准入。定格的读连接和文件句柄留在 `ReleasedOwners` 里；放开之后的回执核对、导出、整本验链、登记和暂存封存都只读定格那一刻，受放开时另给的截止时间约束（生产者给 5 分钟，不超过封存正文保留期），不再挡写。内容截止时刻在放开之前取。

取许可之前先把登录库自己补得掉的待办补完（`checkpoint/settle.rs` 的 `settle_auth`，BUG-044）：补记待发的审计事件（启用两步验证时经 `FactorRuntime::flush_audit`，否则经 `SessionManager::flush_audit`），再跑一轮保留期维护，最多 4 轮，和整个窗口用同一个截止时间。别的库各有周期任务替它们清，登录库的补记平时只在登录、吊销时顺带做，所以由要求它清空的封存入口先清。补记要取完整操作许可，必须在排空之前做。补完到登录库冻结之间仍有一小段窗口（普通的会话检查不取许可就能让新的过期、拒绝事件入队）：撞上了这一轮照旧在 Auth 失败、下一轮再试，不在这里重试。封存卡住时人侧日志写明是哪个 owner、是没排空、超时还是不可用（「备份封存卡在：…」），账本里记的原因不变。

补完登录库、挡住接入方之前，再等人侧空下来（`checkpoint/idle.rs`，BUG-056，2026-11-24）：人侧的完整操作有长有短，一轮同步平时十几秒、外网不通时一分多钟，而排空人侧时接入方已经被挡住。等的办法是每 5 毫秒看一眼（和排空自己的重试一样勤；`AuditCheckpointAccess::idle`：试拿一次独占许可，拿得到当场还掉、不留租约，审计库不健康当场报不可用），等的时候两侧的操作都照常进来；用的是同一个截止时间，到点报「owner Audit 在预算内未排空」，接入方没被挡过。生产者在算冻结预算之前已经用 `await_human_idle` 等过一次（上限 5 分钟，见 `backup-producer.md`），这里补的是那之后又起的慢操作。看到空到真排空之间仍有一小段（平时几毫秒，接入方排空慢时更长）：撞上了照旧在排空人侧那一步用冻结预算等，不重试。两侧的排空等满预算报「在预算内未排空」，审计库自己出错才报「不可用」。

全部冻结后核对各 owner 持久 ACK 的完整原事件；Transport 的最后运行事件、AgentStore 的读取派生索引也使用各自实际原链。窗口里核本作业 Intent、Bridge 与 Transport；写者只定格两棵树、复核一遍水位和元数据，不读回执：它的重放与回执随编辑数涨（169 上核一遍 11 秒），放开之后在从定格导出的副本上核，用冻结那一刻的账本，核的内容和强度不变，核完才有写者的票据（BUG-050，2026-11-24，见 `broker-checkpoint.md`「在副本上核」）；Bridge 每次写前写后都要求 WAL 截断干净、截不掉就封掉实例，放开之后不能留着它的定格读事务，所以 Bridge 库连同传输、平面文件在窗口里整份导出（库有页数上限，是固定大小的状态）。Control、Auth、AgentStore 在窗口里只定格：它们的排空与结构校验（整表计数、`quick_check`、逐条解码回执、逐行核类型与实例）随表变大，放开之后先在定格上做（`validate`，得出水位），再对着定格的库和定格的账本核回执；冻结窗口里不再因为它们没排空而失败，失败挪到放开之后、照样不出票。计数只增加真实核验过的引用及本作业 Intent，不把链中没有持久 ACK 引用的直接事件虚算为 ACK；Audit 自身仍完整验证整条链。

固定目标布局包含 canonical、writer-data、control、auth、bridge、transport、plane、agent、policy、audit、agent-audit。各 owner 只写自己的独立目标，Audit 主水位显式带附属链截止点。除 Bridge 之外的导出都在放开之后从定格做；账本副本整本验链、锚点必须等于冻结时那一个，写者副本从句柄拷、拷前拷后逐个 fstat 核对没被原地改过（改了报 `CaptureChanged`，不出票）。`CheckpointExports` 只是中间结果，不是可交给 worker 的最终 SealTicket。inclusion、整体同步与原子封存必须在此后完成。

## 封存路上的进程内锁

冻结在排空完整操作之后进行，但有些读取不取完整操作许可就拿进程内的锁（每次几毫秒）。以前冻结用 `try_lock`，撞上就整轮封存失败、白等一个间隔（BUG-055，2026-11-24 真机：「备份封存卡在：owner Control 不可用」）。逐处核过：

| 锁 | 谁会短暂拿着 | 现在 |
|---|---|---|
| 黑板引擎锁（`BoardPlane::checkpoint_source`） | 租约巡检、别的黑板请求 | 不碰了：控制库的固定源打开时就取好存在锁外，封存只核一次再克隆 |
| 写者的 `control`、`owner`（`CanonicalWorkbench::freeze_checkpoint`） | 网页读笔记、监视回调、就绪检查 | 最多等 1 秒（`bounded_lock::BRIEF`，不超过冻结截止）；到期、中毒照旧按原类别失败 |
| Bridge 的 `transport`、`bridge`（`SyncPlane::freeze_checkpoint`） | 现在没有：持有者都在同步轮次里，轮次（`run_round`）先取完整操作许可，排空之后没人拿着 | 写法同上，预防性：以后多出短暂持有者也不至于整轮失败 |
| 策略参与者的 `lease`（`policy_checkpoint.rs`） | 没有外人拿（冻结、放开、导出三步都是同一个线程） | 原来就按截止时间等，行为不变，改用同一个函数 |
| `FrozenSyncPlane` 自己的 `frozen`、`files` | 没有外人拿 | 不动，照旧 `try_lock` |
| 各 SQLite 库的写锁、操作门 | 各库自己的写入 | 不动：原来就按截止时间等 |

等 1 秒是上限不是常态：到期就说明有人卡住了，照旧失败，不把冻结预算耗在一把锁上。

108 `20261113-143117-dcb2`：Policy 3（含全局期限）、Worker 原回归 13、严格 Clippy、架构 62 全过；最后仅修正测试统计和拆分夹具，`20261113-143231-47d6` 的总窗口/工作台定向 7 全过。夹具实际装配 Broker、Control、Auth、AgentStore、Policy、Audit 六个 owner，以及独立双账本；Bridge 明确 absent，无模拟平台替身。覆盖全部 SQLite 写屏障持续到 drop、双截止点、真实三 owner 最小装配、缺 Intent 逆序释放、接入方在途操作阻断、重复目标拒绝。真实 Bridge cgroup 整体验收、启动接线、原子 staging、独立 worker 和完整恢复流程仍未完成。
