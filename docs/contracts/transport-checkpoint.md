# Transport checkpoint：原 owner 与固定状态副本

`SyncTransportOwner::freeze_checkpoint` 移入实际 owner，冻结窗口持有原 publisher、排他租约、审批及运行时绑定；`thaw` 返还同一实例。窗口没有 prepare/start/recover 入口，不能另开一个存储实例假装暂停了原 owner。

每次检查重新读取持久状态并复验原绑定。活动槽一律拒绝；存在最后终态时必须通过原实例的 Closed/Confirmed 静止票据复验。没有终态仅接受 generation=1、next_sequence=1 的真实首次状态。取消准备留下的 Unconfirmed 不视为静止证明。

槽闭合后只保留终态摘要，没有保留 ACK 的 offset/hash。封存从实际摘要、审批预算和运行时绑定重建最后运行的 intent/outcome 完整事件；可信组合根必须在 FrozenAudit 原链中查到逐字段相同的事件，不能伪造 ACK，也不能只核对 event_id。核对前清除旧证明，前后源状态必须一致。

实际 FilePublisher 仅将固定 state.json 和新的空 transport.lock 导出到独立、0700、空目录。原锁仍被持有；副本为新建 0600 文件，不覆盖、不硬链接、不清理源。路径和物理祖先均不得重叠；拒绝发布暂存件，保留 2 GiB 空闲下限，逐文件及目录刷盘，重开目标原始状态并核对源、目标绑定及原截止时间。其他 publisher 默认不支持导出。

108 作业 `20261113-140216-3300`：4 项 checkpoint、10 项原存储测试、严格 Clippy、61 项架构检查通过。运行状态测试使用明确的模拟受控后端；文件导出测试使用真实文件和原租约。实际 SyncPlane 门禁、辅助状态及最终七 owner 组合根仍须完成接线，这个子单元不是完整备份验收。
