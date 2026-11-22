# 策略库 checkpoint

T-204（接续 T-147），2026-11-13。策略库机制与真实人侧许可池参与者已实现；
**完整 Rust 备份协调器及启动接线仍未完成**，本页不能作为线上备份或恢复验收。
现有 Python 生产恢复工具是另一条链，已包含策略库，没有被本次代码替换。

`PolicyStore::checkpoint_handle()` 从本句柄的固定源绑定构造封存者；
`PolicyPlane::checkpoint(target)` 把同一库与同一 HTTP 许可池结合为 `PolicyParticipant`。
目标目录由调用方注入，必须预先建成空0700目录；库文件由存储适配器独占创建为0600。
既有文件、符号链接、非私有目录和源身份变化全部拒绝，不覆盖或收养其他文件。

冻结先取得人侧全部许可，等包括审计在内的完整操作结束，再在独立连接上
`BEGIN IMMEDIATE`，持锁至放开（`release`）或 `thaw`。其他进程写者仍等待5秒后返回 `PolicyError::Busy`；
HTTP 为503 `store_busy`，`Retry-After: 5`；MCP说明尚未提交、可稍后重试。
既有库校验采用 Deferred，冻结期间读及读句柄重启不等待写屏障。
上游 AgentStore 的完整接入方调用仍必须先排空；本参与者不能代替该证明。

冻结时（BUG-045，2026-11-22）持锁之后在另一连接执行 PASSIVE checkpoint，必须 `busy=0` 且全部WAL帧已回写；
旧快照读者阻塞时只等到冻结截止时间，不以“快照大概完整”绕过证据。随后经唯一打开点再开一条连接、开读事务读一页，
把库定格在这一刻。`PolicyParticipant::release` 放开写锁、归还 HTTP 许可，定格的读连接留着；之后的导出只受放开时
给的截止时间约束，读的仍是冻结那一刻。导出先在定格的读事务里算全表逻辑摘要，再用 SQLite Backup API 从定格连接
拷进独占创建的目标（不再用 `VACUUM INTO`：它要自己开读事务，在已开着读事务的定格连接上做不了），明确同步原目标FD
和父目录，通过唯一打开点重开并验证schema、quick_check、外键、effective水位与全表逻辑摘要。
草案按rowid排序加入摘要但不比较rowid数值，保留入库先后语义。
最后关闭副本再同步，复核源和目标绑定，所有证据成立才返回封存票据。

截止时间在SQLite VM的progress hook内、行遍历及操作前后核对；取消不留到下一次查询。
系统调用本身（例如fsync）不声称可被VM hook中断，超时后不签成功票据。
冻结/放开/导出串行持有状态锁；请求ID必须与冻结窗口相同，放开只能一次。
解冻先释放SQLite连接再归还HTTP许可（放开时已归还的不重复），失败及Drop也释放；重复解冻无副作用。
失败导出可留下不可用残片，调用方必须使用新目标，机制不递归删除用户目录。

七owner清单 `MANIFEST_SCHEMA_VERSION=2`；v1及未知版本拒绝，不隐式迁移。
当前尚无生产持久Rust清单。空装配只用于实际不存在的策略平面，不能掩盖未实现的参与者。

108证据：`20261113-111905-a47a` 的跨进程/导出8项、人侧参与者2项、
HTTP与MCP Busy各1项、清单23项、架构58项及Clippy通过；
`20261113-112231-ca62` 的VM截止时间中断/后续读恢复1项及Clippy通过。

新根恢复（T-212，2026-11-16，见[新根恢复](new-root-restore.md)）：`PolicyStore::restored_watermark(截止时间)`
按封存时同一套规则重算这个库的 Policy 水位（schema、生效版本、生效哈希、是否全局吊销），只读。编排只开已有库
（`open_existing`），第一次先核水位等于清单里的，再一律全局吊销（2026-11-16 用户拍板），只能由主人经 L3 解除；
封存截止点时已经全局吊销的，原记录保留、不再加一条。AgentStore 的每个实例在这里记实例吊销（不可恢复）。

SQLite语义依据：[Online Backup API](https://www.sqlite.org/backup.html)、
[WAL checkpoint](https://www.sqlite.org/c3ref/wal_checkpoint_v2.html)、[WAL 读事务快照隔离](https://www.sqlite.org/wal.html)。
