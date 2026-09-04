# docs

面向实现者的技术方案。产品需求与决策以 vault `项目/Studiolo/` 为权威（PRD 分册、ADR、功能清单、roadmap），本目录不重复它们；与 vault 冲突时以 vault 为准并回写本目录。

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

设计工作流在 2026-09-02 完成候选与评委阶段，但综合稿未落盘。本次 M0 接续将已拍板项收口到
[`contracts/decisions.md`](contracts/decisions.md)，实际代码与 Linux 验证状态只以
[`contracts/verify-log.md`](contracts/verify-log.md) 为准。

## 后续在此建立

- 部署清单（M2）
- 恢复手册（M2）
- Control DB 迁移沿革（随 schema 版本变化追加）
