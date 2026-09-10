# docs

面向实现者的技术方案。产品需求与决策以 vault `项目/Studiolo/` 为权威（PRD 分册、ADR、功能清单、roadmap），本目录不重复它们；与 vault 冲突时以 vault 为准并回写本目录。

## m1/ — 本机只读工作台、搜索、导航与文件树

[M1 实现说明](m1/README.md) 记录已实现的 T-006 + T-007：显式只读副本、项目总览、待我
动作、文档原文与行号来源、手动全量刷新，以及全文字面搜索、快速路径切换、大纲 / 出链 /
反链导航；包含纯虚构示例库的启动命令、HTTP Interface、只读约束和当前验证状态。
[搜索与文档导航契约](m1/search-navigation.md) 细列查询语义、同代次定位、截断与错误规则。
新增 [T-010 文件树与只读附件信息](m1/file-tree.md)：空目录、纯附件目录、按目录分页与同代次元信息。
**这些切片已实现不代表完整 M1 完成**；变更流、规则漂移、持久索引和 watcher 仍属后续范围。

当前 M1 工作位于 `codex/m1-readonly-workbench`，尚未提交推送；远端 CI 尚未触发。
T-007 当时通过 macOS 158 项、Linux 166 项 Rust 测试及格式 / Clippy 检查，前端 28 项测试、
lint、Prettier 与生产构建。本轮真实 HTTP 桌面/窄屏搜索导航验收通过，控制台无 warn/error；
第一片测试数量和视觉差异账单独标为 T-006 历史记录，见 [M1 实现说明](m1/README.md)。

[服务器示例运行](m1/server-preview.md) 记录用户指定服务器的运行边界、可校验源码包、
原生路径测试与同端口 SSH 转发；不把提前迁移运行位置视为 M2 远程服务验收完成。
[T-009 Cloudflare 入口](m1/cloudflare-preview.md) 记录已部署的域名门禁与独立鉴权服务；真实邮箱登录及公网功能待验收。

T-010历史验证：macOS222/Linux aarch64 230项Rust、前端39项及格式/生产构建通过；
桌面与窄屏实际文件树交互通过。后续T-011界面与M2结果以各检查点为准，当前尚未提交推送或更新服务器。

## m3/ — 供给面服务器常驻与公网入口

[服务器常驻 `studiolo-agent`](m3/server-agent.md) 记录 T-034：白名单子树只读副本同步、amd64 运行包交叉构建与服务器原生测试、
用户级 systemd 单元、专用隧道新增主机名 `studiolo-agent.maxsong.me`（应用内 bearer 门禁，未加 Access）与实际部署记录。
供给面契约本身在 [`contracts/agent-supply.md`](contracts/agent-supply.md)。
T-036托管副本与系统级部署见 [同步运维](m3/replica-operations.md)；
T-037 HTTP四个只读入口、短命会话及验收状态见 [HTTP文本供给](m3/http-text.md)。

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
不将本轮 M1 的实现与测试结果追写成 M0 当时已具备的能力。

## m2/ — 远程与可恢复

[M2 开发状态](m2/README.md)已验收T-029新main整合、审计词法兼容、等待后撤权及schema拒绝副作用（macOS747/Linux758；前端沿用T-025的125项）。HEAD已d24e56a，继续新鲜度/CAS；保留M1未提交成果及旧服务。
应用会话/TOTP、同步、编辑、备份和恢复逐片验收；不把切片完成宣称为完整 M2 发布。
[持久应用会话契约](contracts/auth-session.md) 定义封闭 Access 身份、独立认证存储、
双期限与吊销补记；T-013当时没有HTTP，后续显式会话路由见T-018、TOTP库接口见v3契约。
[v2保留与迁移](contracts/auth-retention.md) 提供显式维护、固定计划/检查点与旧凭证拒重；本片验证状态见M2记录，不等于生产已接线。
[v3 TOTP因子基础](contracts/auth-totp.md)定义可信预配置登记、会话近期认证、持久防重放/限流与故障发布边界；T-015库接口已验收，后续HTTP见T-024，真实登记仍另验。
[v4恢复码与实际替换](contracts/auth-recovery.md)定义换发、单次消费、新因子确认与全会话吊销；T-016库接口已验收，后续HTTP见T-024，真实秘密交付仍另验。
[可信本地登记工具](contracts/auth-admin.md) 定义严格非秘密配置、环境注入、真实隐藏TTY、no-create打开及显式迁移；T-017已双平台验收，不替主人实际登记或开放HTTP。
[应用会话HTTP](contracts/auth-http.md) 定义显式新组合入口、配对Cookie、严格请求校验和已登记签发；T-018最终验证记录见M2，旧启动模式与线上服务不变。
[安全启动](contracts/auth-startup.md) 定义T-019共享主人配置、既有双库/审计锚点检查和监听前失败关闭；当前验证进度见M2，不自动切换线上入口。
[浏览器会话](contracts/frontend-session.md)、[canonical单页读取](contracts/canonical-read.md)及[固定槽因子运行装配](contracts/auth-factor-runtime.md)分别定义T-021～T-023；[生产界面浏览器记录](m2/session-browser-qa.md)仅使用虚构会话夹具。
[因子HTTP与Unix入口](contracts/auth-factor-http.md)定义T-024固定秘密装配、逐请求来源证明、三条动作及Tunnel接入条件；真实部署验收另计。
[因子显示状态](contracts/auth-factor-state.md)与[账户内因子操作](contracts/frontend-factor.md)定义T-025的快照读取、一次性交付和前端不确定结果处理；[因子浏览器验收](m2/factor-browser-qa.md)记录虚构夹具上的桌面、窄屏和失败场景，整体状态见M2。
[M2发布缺口](m2/release-readiness.md) 区分代码、外部输入与真实运行门槛；不是已经批准的缩减发布范围。
[canonical单写者契约](contracts/canonical-writer.md)定义已验收的T-028根租约、持久配对、水位与未决边界；显式未决协调、流式大附件和HTTP编辑仍属后续。

## 后续在此建立

- 部署清单（M2）
- 恢复手册（M2）
- Control DB 迁移沿革（随 schema 版本变化追加）
