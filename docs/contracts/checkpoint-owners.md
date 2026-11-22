# 七 owner 实际协调窗口

`CheckpointOwners` 从正在服务的 CanonicalWorkbench、SessionManager、BoardPlane、SyncPlane、PolicyPlane 及实际身份/审计成对能力取得固定源。它不打开任意数据库、不初始化缺失组件、不把已装配组件冒充 absent；黑板存在而身份源缺席时直接拒绝。

作业先通过独立排队流程持久 `backup.snapshot` Intent。Worker 与协调器共用 `backup_intent` 唯一映射；排队方核对返回记录完整性和完整事件，协调器最终在实际冻结的人侧账本中查到同一原事件才继续。单纯传入作业 ID 不能绕过落账。

先取得接入方完整操作独占许可，再取得人侧许可，然后按 Broker → Control → Auth → Bridge → AgentStore → Policy → Audit 冻结。Audit 阶段包含接入方、人侧两个独立账本。整个窗口从开始到最后导出使用同一单调截止时间；Policy 后续导出或重试不能刷新该时间。任一失败按逆序释放：先关闭账本及上游存储屏障，最后恢复人侧、接入方准入。

取许可之前先把登录库自己补得掉的待办补完（`checkpoint/settle.rs` 的 `settle_auth`，BUG-044）：补记待发的审计事件（启用两步验证时经 `FactorRuntime::flush_audit`，否则经 `SessionManager::flush_audit`），再跑一轮保留期维护，最多 4 轮，和整个窗口用同一个截止时间。别的库各有周期任务替它们清，登录库的补记平时只在登录、吊销时顺带做，所以由要求它清空的封存入口先清。补记要取完整操作许可，必须在排空之前做。补完到登录库冻结之间仍有一小段窗口（普通的会话检查不取许可就能让新的过期、拒绝事件入队）：撞上了这一轮照旧在 Auth 失败、下一轮再试，不在这里重试。封存卡住时人侧日志写明是哪个 owner、是超时还是不可用（「备份封存卡在：…」），账本里记的原因不变。

全部冻结后核对各 owner 持久 ACK 的完整原事件；Transport 的最后运行事件、AgentStore 的读取派生索引也使用各自实际原链。计数只增加真实核验过的引用及本作业 Intent，不把链中没有持久 ACK 引用的直接事件虚算为 ACK；Audit 自身仍完整验证整条链。

固定目标布局包含 canonical、writer-data、control、auth、bridge、transport、plane、agent、policy、audit、agent-audit。各 owner 只写自己的独立目标，Audit 主水位显式带附属链截止点。导出完成仍持有全部屏障；`CheckpointExports` 只是中间结果，不是可交给 worker 的最终 SealTicket。inclusion、整体同步与原子封存必须在此后完成。

108 `20261113-143117-dcb2`：Policy 3（含全局期限）、Worker 原回归 13、严格 Clippy、架构 62 全过；最后仅修正测试统计和拆分夹具，`20261113-143231-47d6` 的总窗口/工作台定向 7 全过。夹具实际装配 Broker、Control、Auth、AgentStore、Policy、Audit 六个 owner，以及独立双账本；Bridge 明确 absent，无模拟平台替身。覆盖全部 SQLite 写屏障持续到 drop、双截止点、真实三 owner 最小装配、缺 Intent 逆序释放、接入方在途操作阻断、重复目标拒绝。真实 Bridge cgroup 整体验收、启动接线、原子 staging、独立 worker 和完整恢复流程仍未完成。
