# SyncPlane checkpoint：Bridge 与附属状态共同封存

实际 `SyncPlane::freeze_checkpoint` 从原槽移出 `SyncTransportOwner`，保留原 publisher、受控后端及租约。普通 driver 和 quiescence 接缝遇到空槽即拒绝。冻结失败或窗口退出经析构返还原实例；先关闭 Bridge 写屏障，最后恢复传输槽。

生产 `PlaneGate` 复验原审批、canonical 根、新旧 Bridge binding、实际冻结 Transport 的完整 binding 与闭合状态，以及真实 `plane.lock` 下的恢复文件与 stage。它使用原始全局截止时间，不接受最近退出时间或测试布尔值作静止证明。Bridge 库仍持有原 `bridge.lock`。全局操作许可须先排空，七 owner 组合根负责这一先决条件。

辅助文件窗口占有实际 `plane.lock`，不提供 save、接收对象或 GC 入口。恢复状态有 `started_sequence`、待补审计清理或发布暂存件时拒绝；首次未落盘状态保持真实缺席，不在源端补写。stage 必须有合法原索引，全部登记对象逐帧核对原摘要、长度和稳定文件身份；孤儿、接收临时件、未知名字均拒绝且保留现场。固定 4096 对象／512 MiB 总预算、单对象 256 MiB、原截止时间保持有效。

导出只建独立新文件和目录：私有 0700/0600、单链接、NOFOLLOW、原根与目标无路径或物理祖先重叠，空目标且保留 2 GiB 空间。对象用 64 KiB 流式核验复制，文件及目录同步后重开全部目标。恢复状态保存原字节，副本锁为空，原锁一直被持有；前后核对源根、锁、状态及全部对象，不删源或覆盖既有目标。

`FrozenSyncPlane::verify_audit` 复核 Bridge 的全部持久 ACK，并通过原事件映射在实际 FrozenAudit 中查验 Transport 最后运行的 intent/outcome。Bridge 数据库、transport 固定文件、plane-state/stage 三个目标全部通过才返回同一个 Bridge 签封回执。辅助复制结束后再次核对原门禁和 Bridge 源。

封存窗口只定格之后（BUG-045，2026-11-22），别的 owner 的导出挪到放开屏障之后做，SyncPlane 仍整份在冻结窗口里核对和导出，放开时析构：Bridge 每次写前、写后都要求 WAL 截断干净（`mutations.rs` 的 `checkpoint`），截不掉就封掉实例；放开之后再留着定格读事务，同步一写就失败。Bridge 库有页数上限，传输与平面文件也是固定上限的状态，留在窗口里不会让窗口随使用变长。

108 `20261113-141439-0034`：平面契约 37（含新封存 6）、入站 10、Bridge 定向 5、严格 Clippy、架构 62 全过。第一轮导出夹具误用容器小容量 `/tmp`，改用作业 TMPDIR；生产空间门槛未降低。真实文件、原平面租约和对象验证有运行证据；SyncPlane 实际受控 cgroup 后端尚无本轮平台演练，不能把这些测试称为完整生产同步冻结验收。

七 owner 协调器、inclusion plan、原子总封存、独立 worker 和恢复启动接线仍在实施。Bridge 独立 DELETE 副本保留旧物理 binding，普通打开继续拒绝；恢复必须显式验证并重新绑定，不能直接启用旧镜像基线。
