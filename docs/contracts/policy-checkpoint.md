# 策略库 checkpoint

T-204（接续 T-147），2026-11-13。策略库机制与真实人侧许可池参与者已实现；
**完整 Rust 备份协调器及启动接线仍未完成**，本页不能作为线上备份或恢复验收。
现有 Python 生产恢复工具是另一条链，已包含策略库，没有被本次代码替换。

`PolicyStore::checkpoint_handle()` 从本句柄的固定源绑定构造封存者；
`PolicyPlane::checkpoint(target)` 把同一库与同一 HTTP 许可池结合为 `PolicyParticipant`。
目标目录由调用方注入，必须预先建成空0700目录；库文件由存储适配器独占创建为0600。
既有文件、符号链接、非私有目录和源身份变化全部拒绝，不覆盖或收养其他文件。

冻结先取得人侧全部许可，等包括审计在内的完整操作结束，再在独立连接上
`BEGIN IMMEDIATE`，持锁至 `thaw`。其他进程写者仍等待5秒后返回 `PolicyError::Busy`；
HTTP 为503 `store_busy`，`Retry-After: 5`；MCP说明尚未提交、可稍后重试。
既有库校验采用 Deferred，冻结期间读及读句柄重启不等待写屏障。
上游 AgentStore 的完整接入方调用仍必须先排空；本参与者不能代替该证明。

导出在另一连接执行 PASSIVE checkpoint，必须 `busy=0` 且全部WAL帧已回写；
旧快照读者阻塞时只等到截止时间，不以“快照大概完整”绕过证据。
随后用参数绑定的 `VACUUM INTO` 导出，明确同步原目标FD和父目录，
通过唯一打开点重开并验证schema、quick_check、外键、effective水位与全表逻辑摘要。
草案按rowid排序加入摘要但不比较rowid数值，保留入库先后语义。
最后关闭副本再同步，复核源和目标绑定，所有证据成立才返回封存票据。

截止时间在SQLite VM的progress hook内、行遍历及操作前后核对；取消不留到下一次查询。
系统调用本身（例如fsync）不声称可被VM hook中断，超时后不签成功票据。
冻结/导出串行持有状态锁；请求ID和预算必须与冻结窗口相同。
解冻先释放SQLite连接再归还HTTP许可，失败及Drop也释放；重复解冻无副作用。
失败导出可留下不可用残片，调用方必须使用新目标，机制不递归删除用户目录。

七owner清单 `MANIFEST_SCHEMA_VERSION=2`；v1及未知版本拒绝，不隐式迁移。
当前尚无生产持久Rust清单。空装配只用于实际不存在的策略平面，不能掩盖未实现的参与者。

108证据：`20261113-111905-a47a` 的跨进程/导出8项、人侧参与者2项、
HTTP与MCP Busy各1项、清单23项、架构58项及Clippy通过；
`20261113-112231-ca62` 的VM截止时间中断/后续读恢复1项及Clippy通过。

SQLite语义依据：[VACUUM](https://www.sqlite.org/lang_vacuum.html)、
[WAL checkpoint](https://www.sqlite.org/c3ref/wal_checkpoint_v2.html)。
