# 内容变更流（T-206 / ADR-032）

2026-11-13 实施中，依据 T-148 的方案 A。Broker 来源链、成功代次差分、L2 接口及审计已读水位已完成定向验证；
文档面板与驾驶舱接线进行中。整批复查和部署尚未完成。

## 可信来源

`FsVaultBroker::list_changes(limit, cursor)` 返回无正文的 `ChangePage`。行含固定历史 ID、全库提交
序号、逻辑路径、种类、主体、前后 revision、之后字节数、服务器请求时间及改名另一端。删除之后
revision 为 None、字节数为 0。未知旧主体和时间保持 None；不从 mtime 或客户端显示名推断。

- 人写与 Obsidian 摄入：从既有 recorded 请求、成功终态和 L0 路径历史恢复语义。
- 删除、墓碑恢复、改名：绑定既有 typed 终态；改名的源和目标各有一行、共用序号，revision 仍各自绑定路径。
- 批量提交与整批回滚：`commit_batch_recorded` / `rollback_batch_recorded` 接受可信 owner 的
  `MutationMetadata`；黑板先验证身份、项目、claim、独占路径，再固定 `Agent {agent_type,
  instance_id, task_id, attempt_id}`。客户端批次 DTO 不能提交作者或时间。
- 每次整批完成都补 L0；原批次终态在回滚后保留，原提交者与回滚发起者不混用。同一提交幂等键
  不可更换主体；已回滚的提交不会重新安装。未公开提交的回滚只公开最终回滚，不冒充原提交曾成功发布。

单页历史与前端主体闭集增加 `agent`；任务和尝试可为空，但有尝试必须有任务。标识限 1–128 个
ASCII 字母数字或 `_.-`，不接受路径或控制字符。人侧文件操作审计仍只接受既有 Human / Obsidian。

## 存储、分页和清理

writer 状态版本 8；完成新历史/索引之前先可靠升级，旧二进制失败关闭。路径 catalog 版本 4
支持批量引用，版本 5 支持待完成的 feed 退役清单。旧请求及绑定字节不重写。

索引在 Broker 私有 `history/meta-v1/catalogs/feed-v1-<n>.json`。每桶分配 16 个提交序号槽，
每个批次最多 32 路径，因此每桶最多 512 行；未公开批次原地回滚时同槽替换原未公开指针。
非十六进制前缀与路径 catalog 名互斥，复用现有目录身份钉住、原代次核对及原子落盘。
桶只存 `(path, sequence, history_id, retired)` 指针；作者、revision 等从重新核验的来源派生。

读取只发布 `sequence <= committed_through`，每页最多 100 行、至多两个索引桶，额外核验命中
路径的 L0 catalog 和来源。游标固定首次提交上限、新提交不挤动后续页；稀疏窗口可返回空页和
继续游标。顺序是提交序号降序、同序号路径升序；时间只供显示，不拿不可靠时钟排序提交事实。

未决安装或批次在途保持失败关闭；篡改指针、路径绑定、终态或作者时返回错误。没有旧桶只能
说明旧历史不完整，返回 `history_incomplete`，绝不说明「没有变化」。首次上线不后台扫描重建
旧历史，不凭旧磁盘时间补发历史行。

`prune_history` 仍要求同时超过 50 版和 30 天。它先把要退役的 `(sequence,id)` 与路径索引一起
可靠保存，再更新 feed 墓碑，最后删除无保留引用对象、清空待办。中断后可续办；读者识别待办
或墓碑，不重新暴露已退役行。批量回滚仍引用的版本沿原保护规则保留。

内存 adapter 提供相同 `list_changes`、批量 recorded、`list_history` / `read_history` 的逻辑契约；
不把它当成文件持久性证明。

## 现有检查

- Broker batch / L0 feed：主体重放、序号可见性、崩溃续办、回滚、跨路径分页、追加时稳定游标、
  篡改拒绝、缺桶提示、prune中断退役与Memory/Fs同契约。
- typed删除恢复和改名既有契约；200 MiB 改名投影仍满足原内存上限。
- 真实黑板入口：调用成功后的私有路径历史固定已认证实例与有效 claim 的 task/attempt。
- 前端历史闭集与展示接受 Agent、拒绝缺失/多余/不成对或非法字段。

## 成功代次观察

canonical 捕获的 Markdown 路径保留 revision 和字节数。只在整份候选通过重验并成功发布时，
与上一成功代次比较；失败捕获、无效 UTF-8 和发布前失效不推进观察记录。只有本提交区间内
Broker 已知行的路径和结果 revision 均匹配，才省略重复观察。解释扫描最多 64 页/4096 行，
不完整时发 `explanation_incomplete` 缺口，不能猜作者。

观察环最多 4096 条，独立递增序号；首次发布仅发 `startup` 缺口，淘汰记录由分页返回
`retention` 缺口。每个进程有新 UUID epoch，旧进程水位不能压住新启动缺口。S2 只覆盖 capture
实际给出 revision 的 Markdown；附件的可靠 Broker 提交仍由 S1 提供，不从附件大小推测内容变化。

## L2 HTTP 与分页

- `GET /api/v1/changes?limit=50&project=项目/名称&before=<opaque UUID>`：limit 1–100；project
  可省略表示全库，只接受两个完整逻辑路径段。可带 `vault_uuid` 核对当前绑定库，省略时由唯一
  owner 返回库身份。未知字段、重复查询字段及非法值拒绝。
- 响应封闭字段：`vault_uuid, project, entries, next_cursor, snapshot, through, history_incomplete,
  retired, seen_complete`。through 为 `{sequence, epoch, observation, generation}`，所有 64 位
  数值均为十进制字符串，未知时间/版本/大小为 null。条目区分 `source=broker|observation`。
- 保留两种来源各自的序号顺序，只用头条可用时间决定下一来源；时间回拨不能重排同源记录。
  时间是显示提示，混合列表不声称全局严格时间顺序。
- 游标内存缓存最多 64 项、存活 10 分钟。绑定主体哈希、Vault、项目、固定提交上限和观察上限；
  保留预取但未消费的 Broker 行（最多 100），不把未显示的行跳过。每次请求至多补取两页 Broker、
  消费 300 条候选；稀疏范围可为空页加 next_cursor。过期/淘汰/重启返回 409，重新读取第一页。
- 每次请求在同一 canonical owner 内先后重验会话与根绑定；缓存不保存授权结果。保护路径不出现在
  列表中。列表在审计不可用时仍可读，已读状态变为未知。没有额外文件或数据库写者。

## 已读审计

`POST /api/v1/changes/seen` 必须同源、L2 与 CSRF，闭集 DTO 是
`{vault_uuid, project, snapshot, request_id}`；后两者均为 UUID。snapshot 引用 GET 签发的上限，
客户端不能直接提交未来序号。操作含义是把该范围的这份快照整体标已读，不只是当前页。

动作名 `content.changes_seen`，同一个稳定事件 ID 先 Intent 后 Outcome；载荷固定 schema 1、
主体哈希、库、范围、快照及四项水位。实际 append 返回事件必须完全一致并通过完整性核对，
才答复 seen。Intent 单独存在不算已读；Outcome 故障可使用同一 request_id 重试，已有 Intent
允许在游标过期后续办。请求编号不能换快照或范围；审计故障返回 503。

全库和各项目分别记水位，同一可信主体跨设备共用，不依赖浏览器 localStorage。读取先有界定位
审计尾部，再扫描最近至多 8192 条，每页至多 1024；只认相同载荷的成对 Intent/Outcome，并取
水位最大值，较旧快照不会在已观察范围内倒退已读状态。尾扫未涵盖整个账本时 `seen_complete=false`：
已证明覆盖的行可以显示已读，其他行保持未知，不报告伪造的零未读或完整总量。进程重启只保留
Broker 水位的意义，观察水位按 epoch 隔离。

## 接口定向检查（108）

`20261113-122447-ca1a`：changes 10 项、真实协调器发布 6 项、真实 HTTP 2 项、架构 58 项和
local-server 全目标严格 Clippy 通过。覆盖固定分页上限、不丢预取行、跨设备与项目隔离、CSRF、
伪造未来值拒绝、Intent/Outcome 故障重试、审计离线仍可读、观察淘汰与游标过期。
