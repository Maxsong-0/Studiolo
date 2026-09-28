# docs

面向实现者的技术方案。产品需求与决策以 vault `项目/Studiolo/` 为权威（PRD 分册、ADR、功能清单、roadmap），本目录不重复它们；与 vault 冲突时以 vault 为准并回写本目录。

**看状态先看这两处**：仓库总口径在根 [README](../README.md)「当前状态」，
逐工作域的验收台账在 [M2、M3 整体收尾](m2/m2-m3-closeout.md)。本目录里的分片文档
大多保留**写作当时**的测试数字与结论，是历史检查点，不代表今天的状态。
台账本身也是快照——它写于 **2026-09-12（T-046）**，**不覆盖 T-047～T-063**；
这三项看根 README 与各自文档，其余工作域以台账为准。
**没有实跑证据的一律写「未验证 / 未验收」**，不用「构建成功」代替发布证据。

本页是索引：每行一句话说明该文件是什么，不复制其中的结论。

## handoff/ — 给读不到 vault 的会话的交接快照

| 文件 | 内容 |
|---|---|
| [`handoff/README.md`](handoff/README.md) | vault 里在推进工作的文档快照（vault 为准）、云端会话用 GitHub CI 验证与「交本机会话」的边界；当前是 T-160 上线续做 |

## design/ — 界面设计规范

| 文件 | 内容 |
|---|---|
| [`design/t130-design-system.md`](design/t130-design-system.md) | T-130 瑞士网格＋工程图纸设计规范（2026-09-22 用户定稿），取代 T-011 |

## m1/ — 本机只读工作台、搜索、导航与文件树

已实现 T-006 总览、T-007 搜索导航、T-010 文件树、T-011 界面重设计；
**这些切片已实现不代表完整 M1 完成**，变更流、规则漂移、持久索引和 watcher 仍属后续范围。
`studiolo-local-server` 现已同时承担 M2/M3 的模式，M1 文档只描述其只读入口。

| 文件 | 内容 |
|---|---|
| [`m1/README.md`](m1/README.md) | M1 实现约定、只读入口启动方式、运行契约、四个只读 Interface，以及 T-006～T-011 的历史验证记录 |
| [`m1/search-navigation.md`](m1/search-navigation.md) | T-007 查询语义、同代次定位、截断与错误规则 |
| [`m1/file-tree.md`](m1/file-tree.md) | T-010 空目录、纯附件目录、按目录分页与同代次元信息 |
| [`m1/design/t011-design-system.md`](m1/design/t011-design-system.md) | T-011 Apple 通透工作台视觉规范（**已被 [T-130 规范](design/t130-design-system.md) 取代**，仅作历史参考）；曾取代旧 `overview-concept.png` 的历史视觉约束 |
| [`m1/server-preview.md`](m1/server-preview.md) | T-008 服务器示例的运行边界、可校验源码包、原生路径测试与同端口 SSH 转发 |
| [`m1/cloudflare-preview.md`](m1/cloudflare-preview.md) | T-009 域名门禁与独立鉴权服务；已部署生效，真实邮箱登录与公网功能**仍待验收** |
| [`m1/ui-release.md`](m1/ui-release.md) | T-035 界面与动效的完善、合并与服务器 4319 网页切换记录 |

「服务器已运行」不等于 M2 远程服务验收完成；该目录里的测试数字都是各分片当时的历史结果。

## m2/ — 远程与可恢复

M2 包含真实人侧身份、CAS 编辑与 L0、双向 Obsidian Sync、附件 / 冲突 / 删除保护、
一致备份恢复、健康告警和实际两周 headless 试跑。**完整 M2 未完成、未发布**；
分片通过不等于里程碑通过。下表标「T-048 实施中」的是本轮收尾正在写的代码，**尚未验收**。

### 状态与缺口

| 文件 | 内容 |
|---|---|
| [`m2/m2-m3-closeout.md`](m2/m2-m3-closeout.md) | 逐工作域的事实与收口条件、验证证据、外部接入准备；**T-046 之前各域的权威台账，写于 2026-09-12，未覆盖 T-047～T-063** |
| [`m2/README.md`](m2/README.md) | M2 分片开发顺序与 T-012～T-029 的逐片验证记录（历史检查点） |
| [`m2/release-readiness.md`](m2/release-readiness.md) | 2026-09-05 的发布缺口快照；其中的「尚未实现」已被台账取代，不作当前状态 |
| [`m2/connection-setup.md`](m2/connection-setup.md) | 外部接入信息（远端 Vault、NAS、告警邮箱）现在各自核实到哪一步 |
| [`m2/m0-review-integration.md`](m2/m0-review-integration.md) | M0 复审修复合入 main 的整合记录与兼容边界 |
| [`m2/auth-design.md`](m2/auth-design.md) | 会话 / TOTP / 恢复码的分片实施设计与授权矩阵 |

### canonical 写入、新鲜度与人侧正文

| 文件 | 内容 |
|---|---|
| [`m2/canonical-write-design.md`](m2/canonical-write-design.md) | 单写者与未决幂等的设计输入（含旧实现错误 CAS 的真实复现） |
| [`m2/canonical-freshness-design.md`](m2/canonical-freshness-design.md) | 新鲜度的后续设计输入；复用完整扫描与读模型，不另造索引系统 |
| [`m2/local-observation-contract.md`](m2/local-observation-contract.md) | 完整本地观察内核：已实现的观察单元与接口边界 |
| [`m2/human-save-design.md`](m2/human-save-design.md) | 人侧保存的设计输入：L2 正文与 M3 的 L3 policy 分界 |
| [`m2/human-content-contract.md`](m2/human-content-contract.md) | 人侧正文编辑契约（范围、生成区、新鲜度与 CAS） |
| [`m2/streamed-content.md`](m2/streamed-content.md) | 流式附件、L0 与恢复；不证明真实 Sync 额度或设备往返已接通 |
| [`m2/writer-maintenance.md`](m2/writer-maintenance.md) | `studiolo-maintain`：只处理一项未决事务及其固定审计，不做全库恢复 |

### 整文件操作与重命名

| 文件 | 内容 |
|---|---|
| [`m2/file-operations-design.md`](m2/file-operations-design.md) | CAS 删除、最近删除与安全重命名的原始设计 |
| [`m2/file-operations.md`](m2/file-operations.md) | 已接入代码的整文件删除与最近删除恢复 |
| [`m2/file-operations-config.md`](m2/file-operations-config.md) | 文件操作的私有批准配置；配置存在不代表服务已启用 |
| [`m2/file-rename-contract.md`](m2/file-rename-contract.md) | 复合 Rename 的实施契约与版本 / API 登记 |
| [`m2/human-rename-contract.md`](m2/human-rename-contract.md) | 人侧 Rename 的封闭配置与共享生成区保护 |

### 同步、隔离与 Bridge

| 文件 | 内容 |
|---|---|
| [`m2/sync-transport-design.md`](m2/sync-transport-design.md) | 双向 Sync 与镜像写入协调的设计输入；**尚未接入真实 Sync** |
| [`m2/sync-scope.md`](m2/sync-scope.md) | 整文件同步范围的私有配置与范围核对；不是实际 SyncTransport |
| [`m2/headless-execution-contract.md`](m2/headless-execution-contract.md) | Obsidian Headless 0.0.14 的静态核查；**未安装、未运行、未登录、未同步真实 Vault** |
| [`m2/owned-run-design.md`](m2/owned-run-design.md) | OwnedRun 的实施候选与 Linux 静止证明要求 |
| [`m2/owned-run.md`](m2/owned-run.md) | T-048 SyncTransportOwner 实施记录；**T-048 实施中，无受控隔离即 `UnsupportedContainment`** |
| [`m2/bridge-planner.md`](m2/bridge-planner.md) | 双向 Bridge 三方规划契约；仅元数据规划，未持久化、未接真实同步 |
| [`m2/bridge-state.md`](m2/bridge-state.md) | Bridge 持久状态契约 |
| [`m2/bridge-executor.md`](m2/bridge-executor.md) | MirrorCAS 与双向 Bridge 执行器顺序核；不做任何 IO，**生产未接线** |

### 备份、恢复与运维

| 文件 | 内容 |
|---|---|
| [`m2/checkpoint-recovery-contract.md`](m2/checkpoint-recovery-contract.md) | 一致 checkpoint 与新目录恢复契约（冻结实现边界） |
| [`m2/backup-worker.md`](m2/backup-worker.md) | checkpoint 协调器、独立 BackupWorker 与恢复 kit 的实施记录；**T-048 实施中，未验收** |
| [`m2/health-and-alerting.md`](m2/health-and-alerting.md) | 健康页、配额、白名单诊断、带外告警与死人开关；**T-048 实施中，生产未启用** |

### 浏览器验收（均使用虚构夹具）

| 文件 | 内容 |
|---|---|
| [`m2/session-browser-qa.md`](m2/session-browser-qa.md) | T-021 会话界面；无真实身份、Cookie 秘密、Vault 或公网接入 |
| [`m2/factor-browser-qa.md`](m2/factor-browser-qa.md) | T-025 因子界面；无真实 Access 或 HTTPS Cookie |

配套契约在 `contracts/`：[会话](contracts/auth-session.md)、[v2 保留与迁移](contracts/auth-retention.md)、
[v3 TOTP](contracts/auth-totp.md)、[v4 恢复码](contracts/auth-recovery.md)、
[登记 CLI](contracts/auth-admin.md)、[会话 HTTP](contracts/auth-http.md)、
[安全启动](contracts/auth-startup.md)、[因子 HTTP 与 Unix 入口](contracts/auth-factor-http.md)、
[因子运行装配](contracts/auth-factor-runtime.md)、[因子显示状态](contracts/auth-factor-state.md)、
[前端会话](contracts/frontend-session.md)、[账户内因子操作](contracts/frontend-factor.md)、
[前端视图位置](contracts/frontend-navigation.md)、
[canonical 单页读取](contracts/canonical-read.md)、[canonical 单写者](contracts/canonical-writer.md)、
[canonical 新鲜度](contracts/canonical-freshness.md)。

## m3/ — 供给面、资料同步与图 2

M3 包含有界冷启动读取、MCP / HTTP 类型与实例身份、白名单及读取审计、资料同步 / 新鲜度、
客户端投影实装、持久派生索引，以及图 2 的目录职责 / 漂移、声明架构、依赖与人工设计路径。
T-033～T-040 已合并 main 并上线；**T-041～T-046 已随 PR #5 合并 `origin/main`（`297f126`）
但没有部署、没有整体验收**，线上跑的仍是 T-040 版本；**T-048、T-063 只在收尾分支上，
未合并、未部署**；**真实客户端安装与图 2 生产绑定未完成**。

| 文件 | 内容 | 发布状态 |
|---|---|---|
| [`m3/server-agent.md`](m3/server-agent.md) | T-034 `studiolo-agent` 常驻：只读副本同步、amd64 运行包、systemd 单元与专用隧道主机名 | 已上线 |
| [`m3/replica-freshness.md`](m3/replica-freshness.md) | T-036 资料副本同步协议与 MCP 新鲜度 | 已上线 |
| [`m3/replica-operations.md`](m3/replica-operations.md) | T-036 启用与回滚的操作步骤（步骤本身不是已启用的证据） | 已上线 |
| [`m3/http-text.md`](m3/http-text.md) | T-037 四个 HTTP 只读入口、短命会话与同一授权 / 审计 / 新鲜度 | 随 T-040 上线 |
| [`m3/client-projection.md`](m3/client-projection.md) | T-038 Claude/Codex 独立资料目录、显式更新与漂移检查 | CLI 已发布，**未安装到真实客户端** |
| [`m3/persistent-index.md`](m3/persistent-index.md) | T-039 单篇解析持久缓存、未变模型共享与隔离边界 | 随 T-040 上线 |
| [`m3/read-supply-release.md`](m3/read-supply-release.md) | T-040 三片合并、完整运行包、系统服务升级与回滚 | 已上线 |
| [`m3/project-operations.md`](m3/project-operations.md) | T-041 显式本地仓库绑定、目录职责图与双向漂移 | 已合并 main，未部署 |
| [`m3/architecture-flow.md`](m3/architecture-flow.md) | T-042 有来源的模块关系、数据流与逐步设计路径 | 已合并 main，未部署 |
| [`m3/cargo-dependencies.md`](m3/cargo-dependencies.md) | T-043 显式启用的 Cargo 包关系、依赖条件与清单来源 | 已合并 main，未部署 |
| [`m3/rust-call-sites.md`](m3/rust-call-sites.md) | T-044 显式 Rust 文件白名单、词法上下文与未解析目标的调用位置 | 已合并 main，未部署 |
| [`m3/rust-call-links.md`](m3/rust-call-links.md) | T-045 同文件自由函数的受限词法关联与反向引用 | 已合并 main，未部署 |
| [`m3/mcp-session-boundaries.md`](m3/mcp-session-boundaries.md) | T-046 MCP 会话身份连续性、凭据轮换与资源边界 | 已合并 main，未部署 |
| [`m3/repository-approval.md`](m3/repository-approval.md) | T-046 图 2 生产仓库的私有配置批准契约 | 已合并 main，未部署；**真实生产配置未验收** |
| [`m3/release-packaging.md`](m3/release-packaging.md) | T-063 local-server 运行包内容、本机出包与自检、服务器安装与回滚步骤 | 仅开发分支（文件尚未提交）；**真实服务器部署与公网复验未做**，以该文件「已验证与未验证」一节为准 |

供给面契约本身在 [`contracts/agent-supply.md`](contracts/agent-supply.md)。
`m3/client-projection.example.json` 是投影输出的示例，不含真实资料；
`m3/client-projection.policy.example.json` 是从策略平面 effective 导出的配置示例（T-111）。

## m4/ — 策略平面与权限中枢

M4 把规则母本、skills 与各 AI 类型的能力集升入策略平面：desired → 校验 → diff（扩大项标红）→ L3 One-time Code → effective，
版本与回滚，实例／类型／全局三级吊销。**代码已实现；macOS 全工作区 182 目标 2293 项通过，Linux 以 PR #10 的 CI（x86_64）为准；线上未启用**（agent 未给 `--policy`，人侧因子模式未部署）。

| 文件 | 内容 | 发布状态 |
|---|---|---|
| [`m4/README.md`](m4/README.md) | T-069 实现范围、本机运行方式、二元验收证据、明确未做与启用前置 | 已在开发分支，未部署 |
| [`contracts/policy-plane.md`](contracts/policy-plane.md) | 策略平面契约：私有库、三类母本形状与恒定约束、发布链路与审计顺序、三级吊销、接入方视角、错误模式、契约测试清单 | 同上 |

## contracts/ — Module Interface 契约（M0）

六个深 Module 的 Interface、不变量、错误模式、测试替身与契约测试清单。范围与关键裁决的
来源集中记录在总览和 `decisions.md`；代码以这些文件为契约，实现与契约不一致时先记录
差异和裁决，再同步改契约与代码。

| 文件 | 内容 |
|---|---|
| [`contracts/00-overview.md`](contracts/00-overview.md) | crate 划分与依赖方向、同步 / 线程模型、错误分类总则、M0 真实实现与替身的划分、canonical Vault 写入口清单与架构测试 |
| [`contracts/core-types.md`](contracts/core-types.md) | 跨模块领域类型及 M0 代码归属：ID、Revision、Snapshot、Fence、Capability、分级枚举与错误分类 |
| [`contracts/vault-broker.md`](contracts/vault-broker.md) | VaultBroker：canonical Vault 唯一写者 |
| [`contracts/vault-index.md`](contracts/vault-index.md) | VaultIndex：解析、就近 wikilink、反链、标题树、frontmatter 归一 |
| [`contracts/task-engine.md`](contracts/task-engine.md) | TaskEngine：命令、claim / attempt / fence、取消、projection outbox、reconcile |
| [`contracts/board-overview.md`](contracts/board-overview.md) | T-130 跨项目黑板只读汇总 `GET /api/v1/board/overview`：查询参数与 400 答复、鉴权与许可、字段与截断、一致性约束、前端封闭解码器 |
| [`contracts/gateway.md`](contracts/gateway.md) | Gateway：capability 校验、权限求交、call 幂等、intent / outcome 顺序 |
| [`contracts/audit-ledger.md`](contracts/audit-ledger.md) | AuditLedger：权威审计唯一写入口、完整性链 |
| [`contracts/runner-adapter.md`](contracts/runner-adapter.md) | Runner Adapter：Stub 与真实 CLI 共用的执行器 Interface |
| [`contracts/control-db.md`](contracts/control-db.md) | Control DB schema、事务边界、迁移、`.workbench/vault.json` |
| [`contracts/vertical-chain.md`](contracts/vertical-chain.md) | M0 最小垂直链时序、重启后 reconcile 场景、三项 M0 退出测试 |
| [`contracts/decisions.md`](contracts/decisions.md) | 设计取舍记录：采用 / 放弃 / 理由 |
| [`contracts/verify-log.md`](contracts/verify-log.md) | 实际验证日志与尚未执行项；不得用计划代替通过证据 |
| [`contracts/vault-update-notes.md`](contracts/vault-update-notes.md) | M0 权威退出标准与真实 vault 档案的回写对照清单 |

设计工作流在 2026-09-02 完成候选与评委阶段，但综合稿未落盘。M0 接续将已拍板项收口到
[`contracts/decisions.md`](contracts/decisions.md)，M0 当时的代码与 Linux 验证状态以
[`contracts/verify-log.md`](contracts/verify-log.md) 为准。该目录保留历史 M0 契约和验收语义，
不将后续里程碑的实现与测试结果追写成 M0 当时已具备的能力。

## 后续在此建立

- M2 部署清单与恢复手册（`deploy/` 目前只有 M3 agent 单元与 M1 示例模板）
- Control DB 迁移沿革（随 schema 版本变化追加）
