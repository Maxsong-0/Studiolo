> 同步来源：vault `03-记忆/ADR-031-策略checkpoint使用SQLite写屏障.md`，2026-11-15 快照（vault 为准，可能滞后）。

# ADR-031 策略 checkpoint 使用 SQLite 写屏障

日期：2026-11-13。状态：实施中。来源：[[2026-10-11-T-177c-拍板清账]] A3 用户已选择；[[2026-11-13-T-204-策略checkpoint接线实施记录]]。

采用专用连接 BEGIN IMMEDIATE 形成跨进程写屏障，独立连接 PASSIVE checkpoint 和 VACUUM INTO 到新0600文件；目标目录由调用方构造注入。正常写者仍等待5秒后Busy。既有库只读校验改Deferred。

导出前后核对固定源绑定；重开副本验证格式、完整性、外键、水位与全表逻辑摘要。草案表摘要按rowid排列但不纳入rowid数值，保留发布先后语义。文件和父目录明确fsync；失败不签票据。manifest版本升2。

SQLite文档确认 VACUUM INTO 不改源文件，但可能重编号未声明INTEGER PRIMARY KEY的rowid；当前版本在synchronous NORMAL/FULL下同步输出，仍执行项目要求的显式文件与目录同步。来源：[VACUUM](https://www.sqlite.org/lang_vacuum.html)、[checkpoint](https://www.sqlite.org/c3ref/wal_checkpoint_v2.html)。旧提案“不fsync”描述不再作为事实。

SQLite写锁只证明事务排空，跨owner调用与审计投递仍必须由真实参与者/协调器排空；不把未实现参与者标absent。真实startup接线和验证证据完成后更新状态。
