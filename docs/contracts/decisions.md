# M0 Interface 设计决策

> 状态：2026-09-03 综合收口，2026-09-04 根据实现与对抗校验修订。此表补齐旧设计工作流
> 未落盘的综合结论；若实现发现不可行，
> 先在本文件记录替代方案与证据，再同步修改对应契约和测试，不能让代码静默偏离。

| ID | 决定 | 放弃项与理由 | 依据 |
|---|---|---|---|
| M0-D01 | 六个深 Module、九个 workspace crate；Control DB 与 core 是支撑，不虚增第七个 Module | 沿用 PRD 旧“五 Module”会让五套面板解析各自漂移 | roadmap M0；功能清单“跨面板底座” |
| M0-D02 | M0 外部 Interface 同步；异步运行时留在未来 Adapter/组合层 | 现在引入双套 sync/async trait 会扩大 Seam，且 M0 无网络 runtime | M0 实施计划待定项的本轮收口 |
| M0-D03 | 跨 kind attempt manifest 固定为 `sources/attempts/<task-id>/<attempt-id>/manifest.json` | 放在 web/db kind 下无法代表跨源 attempt；放在 task 正文会把 commit point 与人写内容耦合 | 旧评委 3/3 候选共识；本轮接续指令拍板 |
| M0-D04 | Runner 成功采用 `running -> committing -> needs_review` 两阶段；manifest 是唯一 commit point | report 直接终态会在文件写崩溃时产生“DB 成功、产物不完整” | PRD manifest-last、跨平面对账；本轮拍板 |
| M0-D05 | reconcile 只前滚已持久事实或收敛到 failed/abandoned，绝不启动 Runner | 重启后复活旧 `running` 会重复外部副作用和产物 | PRD 13.2、18.2；本轮拍板 |
| M0-D06 | agent task 字段分成身份、人写定义、待确认意图、TaskEngine 投影四类；`input_snapshot` 明确归投影 | 按最新文件整页回写会覆盖人写字段；把 requested_action 当 command 会伪造授权 | PRD 5.6、6.5；ADR-010 |
| M0-D07 | Vault revision 使用 SHA-256 并绑定 Vault UUID、逻辑路径与原始字节；全库不用 BLAKE3 | 只 hash 内容可跨 Vault/路径误复用；两种 hash 增加协议歧义 | 技术栈最终表；PRD revision 定义 |
| M0-D08 | M0 真实 Adapter 是文件系统 VaultBroker、SQLite Control DB、Stub Runner；Gateway/AuditLedger 用有约束 fake；TaskEngine 有最小持久实现 | 全部做成空壳不能通过重启/垂直链；提前做真实网络/审计文件则越过 M0 | M0 实施计划 5、退出标准 3/5 |
| M0-D09 | VaultIndex 本轮采用类型化读侧方法：全量 `rebuild` 加 document/heading/link/backlink/health/generation 查询；解析/别名/比较函数共用生产逻辑 | 泛化 `apply(change)` + `query(query)` 命令接口留待增量索引出现后做兼容评估；M0 无 watcher 游标，不为假增量建协议 | 功能清单；父任务本轮裁决 |
| M0-D10 | Control DB 单写者是 TaskEngine；ControlDb 事务方法封装状态/fence/outbox 的 SQL 不变量，TaskEngine 决定业务动作与跨模块顺序 | Web、Gateway 或 Runner 持有写连接会绕过 TaskEngine；把 SQL transaction 拆到各调用方又会破坏原子性 | PRD 5.6/5.7、10.5；2026-09-04 source/契约对照 |
| M0-D11 | AuditLedger 在 M0 冻结带版本 domain separation 的 SHA-256 前向链与 event+phase 幂等；磁盘 framing/canonical 字节公式等文件格式后置 | 仅 append Vec 无法测试幂等/链关系；M0 提前冻结尚未实现的文件格式没有退出收益 | PRD 5.7/9.5；技术栈 SHA-256 |
| M0-D12 | M0 Stub 从 snapshot 派生临时产物，但不声称已实现 Demo Web/DB | 固定结论无法证明输入绑定；提前实现 Gateway Demo 属 M3 | roadmap M3；M0 最小垂直链 |
| M0-D13 | 路径负向测试取实施计划、ADR-001 与 PRD 12.7 的并集 | 只测字符串 `..` 或单次 realpath 不满足目录句柄安全 | ADR-001；PRD 12.7 |
| M0-D14 | fake 与真实 Adapter 通过同一 Interface；fake 必须保留拒绝与幂等，故障由可配置 fake 或同 trait failing wrapper 注入 | “永远成功”的 mock 会让调用方绕过 Module 不变量；强迫每个内存实现内建所有故障点会污染生产 Seam | M0 实施计划；Interface 即测试面 |
| M0-D15 | `JobCapability`、Gateway DTO 与只读 `AttemptGuard` 放在 gateway crate，复用 core 的 task/attempt/fence/tool 类型 | Gateway 自有入口不依赖 TaskEngine；共享基础类型与专属权限 DTO 分开，后续迁移应保持 trait 行为兼容 | 本轮 Module 责任归属裁决 |
| M0-D16 | VaultBroker 公开 Seam 拆为 reader/snapshotter/snapshot-reader/writer 四个能力 trait；M0 不暴露 `changes(cursor)` | 单一胖 trait 扩大调用方权限；没有 watcher 时宣称 change feed 会制造虚假 freshness | 当前公开 API；父任务本轮裁决 |
| M0-D17 | manifest 只冻结固定 commit path、身份/摘要/审计关联与 manifest-last；不冻结 private Rust struct 的 JSON 布局 | 让集成测试读取私有字段会把暂存格式误升为公共协议 | 父任务本轮裁决；M0 退出标准 5 |
| M0-D18 | manifest 后的 T2 同时写 task 终态、projection outbox 与 terminal audit outbox，再补 outcome/ack | 若 outcome 在 T2 前声称 needs_review，DB 可能仍未提交；若 T2 后只靠内存补记，重启会丢失审计结果 | 2026-09-04 对抗核对；失败关闭与崩溃恢复不变量 |
| M0-D19 | M0 的受信任进程内 Run 冻结 task 页面和显式 inputs，claim 只加载该 snapshot | 在 Run 后到 claim 之间重新读源会让实际运行输入偏离已确认任务 | PRD snapshot/Run 语义；2026-09-04 主线裁决 |

## 明确后移

- M1：本机只读工作台、持久索引、搜索/看板/图表 UI。
- M2：Web/API、远程入口、认证、Obsidian Sync bridge、编辑与备份恢复。
- M3：完整 TaskEngine/Gateway/AuditLedger、Demo Web/DB、policy 与审计页。
- M4：Claude Code/Codex Adapter、模型 relay、外发与支出闸门。

敏感动作枚举表虽是 M0 文档交付物，其 M2–M4 端点不因核心类型存在而提前实现。

## 仍需实现证据确认的细节

以下不是范围未定，而是实现完成前必须以代码/测试回填证据：

- Rust 公开类型和本契约术语的一一映射；Gateway DTO 的未来迁 core 只做兼容性重构；
- 若未来公开 portable manifest 格式，其 canonical JSON 与 golden fixture 需另行拍板；
- Linux 内核不支持 openat2 时的拒绝行为；当前显式调用不降级，不能把 cap-std 的研究结论
  当成此 Adapter 已实现回退；
- committing 的 artifact bytes 已随 reservation 保存，重启不依赖 Runner 临时目录；损坏计划
  必须失败关闭，其故障收敛行为以 TaskEngine 契约与测试为准；
- AuditLedger 文件 Adapter 的 fsync/轮换格式（不阻塞 M0 内存 Adapter）。
