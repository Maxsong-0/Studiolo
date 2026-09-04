# M0 契约总览

> 状态：M0 设计冻结。产品范围以 vault `项目/Studiolo/` 的 `功能清单.md` 与
> `roadmap.md` 为准；M0 退出标准以归档
> `04-协调/归档/2026-09-04-M0-实施计划.md` 为准。PRD v1.1
> 尚未回写的五 Module、Syncthing 与旧里程碑编号均不得覆盖后续 ADR。

## 1. M0 的完成形态

M0 不是可运行的网页产品。完成物是：

1. 六个深 **Module** 的小 **Interface**、不变量、错误模式、内存测试 **Adapter**
   与契约测试：VaultBroker、VaultIndex、TaskEngine、Gateway、AuditLedger、Runner Adapter。
2. 两个支撑 crate：`studiolo-core` 提供跨 Seam 值类型；`studiolo-control-db`
   提供 SQLite schema、迁移与事务方法，落实 TaskEngine 的状态、fence 和 outbox 不变量。
3. 三处 M0 真实 Adapter：VaultBroker 文件系统 Adapter、Control DB SQLite Adapter、
   Stub Runner。为满足重启验收，TaskEngine 还必须有一个使用真实 Control DB 的最小
   持久实现；完整 M3 调度能力不因此提前。
4. 一条可自动验收的垂直链，以及 Linux 路径安全、重启对账和写入口架构测试。

M0 不提供常驻服务、Web/API、前端、远程部署、同步 bridge、备份、真实 Web/DB、
真实 CLI Runner 或模型中继。Gateway、AuditLedger 与 VaultIndex 的外部 I/O Adapter
在 M0 可由内存替身承担；VaultIndex 的纯解析与归一规则应由生产代码和替身共用。

组合入口 `studiolo::M0Workbench::open(vault_root, data_root)` 装配 FsVaultBroker、绑定同一
Vault UUID 的文件 ControlDb、MemoryAuditLedger 与 StubRunner。它不自动 reconcile 或
start；调用方显式执行生命周期。重新打开会新建内存账本，不能把 M0 组合宣称为持久审计
服务；需要重启恢复的是 Control DB 状态、已封存提交计划与未交付 outbox。

## 2. 术语与 Seam 纪律

- **Module**：对调用方隐藏复杂实现，只暴露一个 Interface。
- **Interface**：除 Rust trait 外，还包括不变量、调用顺序、错误模式、配置与性能约束。
- **Seam**：Interface 所在的位置；生产 Adapter 与测试 Adapter 在同一 Seam 上替换。
- **Adapter**：满足 Interface 的具体实现，不把 Module 内部细节泄漏给调用方。

测试只从 Interface 观察结果；不得为了测试把目录句柄、SQLite transaction、解析器、
审计文件句柄或 Runner 进程句柄的内部形态暴露成公共 Interface。

## 3. crate 与依赖方向

| crate | 允许的 workspace 内部依赖 |
|---|---|
| `studiolo-core` | 无 |
| `studiolo-vault-broker` | core |
| `studiolo-vault-index` | core |
| `studiolo-control-db` | core |
| `studiolo-audit-ledger` | core |
| `studiolo-runner-adapter` | core |
| `studiolo-gateway` | core、audit-ledger |
| `studiolo-task-engine` | core、control-db、vault-broker、audit-ledger、runner-adapter |
| `studiolo` | 组合以上全部 crate，并承载端到端/架构测试 |

硬规则：

- `studiolo-core` 不依赖任何 workspace 成员。
- `studiolo-gateway` 不依赖 `studiolo-task-engine`；M0 的 fence/取消查询通过 Gateway crate
  自己声明的只读 `AttemptGuard` 注入。后续迁往 core 是兼容性重构，不改变依赖方向。
- TaskEngine 决定业务动作和跨 Module 顺序；`studiolo-control-db` 封装该动作所需的 SQL
  transition helpers，在同一 transaction 中强制状态、fence、唯一 attempt 与 outbox
  不变量。它不调度任务、不调用 Broker/Runner，也不向其他 Module 暴露写连接。
- 具体 Adapter 在组合根装配，Module 不在方法内部自行创建外部依赖。
- M0 的外部 Interface 同步执行。未来异步网络/进程 Adapter 在 Module 内部或传输层
  适配，不为此复制一套领域 Interface。

## 4. 权威域与唯一写入口

| 目标域 | 唯一写者 | M0 断言 |
|---|---|---|
| canonical Vault 与 `WORKBENCH_DATA/history/` | VaultBroker 文件系统 Adapter | 只有 `vault-broker` 可依赖 Vault 文件写实现；其他 Module 只能调用 `VaultBroker` Interface |
| `.workbench/vault.json` | VaultBroker 初始化路径 | 普通 mutation 一律拒绝；只初始化一次，Vault UUID 不可被副本替换 |
| Control DB | TaskEngine | 独占 `ControlDb`，只经其事务方法写入；其他 Module 不执行写 SQL |
| `WORKBENCH_DATA/audit/` | AuditLedger | 其他 Module 只能调用 `append`；M0 可仅有内存 Adapter |
| `WORKBENCH_EPHEMERAL/gateway-raw/` | Gateway | Runner 不可见；M0 无真实外部调用 |
| attempt 临时目录 | Runner Adapter | 不在 Vault 内；只有通过 TaskEngine + VaultBroker 的提交才成为持久产物 |

架构测试必须结合 Cargo 依赖图与源码扫描枚举 M0 写入口；仅靠约定或注释不算通过。

## 5. 跨 Module 顺序约束

1. TaskEngine 的 `claim` transaction 是创建 attempt、lease、fence 的唯一线性化点。
2. Gateway 外部调用必须先持久化 AuditLedger intent；写失败则不调用外部目标。
3. Runner 输出只能进入 attempt 临时目录。
4. Runner 成功报告先使 attempt 进入 `committing`，并持久化不可变提交计划。
5. VaultBroker 校验提交计划，逐项幂等落盘，最后原子写
   `sources/attempts/<task-id>/<attempt-id>/manifest.json`。该 manifest 是唯一 commit point。
6. manifest 可验证后，TaskEngine 才在同一 DB transaction 提交 `needs_review`、projection
   outbox 与 terminal audit outbox；之后追加审计 outcome 并确认交付。outcome 不可用时保留
   待补记录，不重写 artifact。
7. `reconcile()` 只前滚已经持久化的事实或收敛到失败/`abandoned`；绝不重新运行 Runner，
   绝不把旧 `running` 直接恢复成 `running`。

完整时序和崩溃矩阵见 [vertical-chain.md](vertical-chain.md)。

## 6. 共同错误规则

- 预期拒绝必须是结构化错误，不得以字符串匹配决定控制流。
- `conflict` 必须携带当前 revision 或生成 diff 所需的最小元数据。
- 相同幂等键/事件 ID/调用 ID与相同请求返回第一次结果；同键不同请求返回
  `idempotency_mismatch`，不得覆盖旧结果。
- `stale_fence`、`cancelled`、`already_terminal` 都是正常业务拒绝，不得重试为成功。
- 外部调用结果不确定时返回 `indeterminate`，不得自动重放。
- 权威审计不可用时，外部工具、策略发布、审批和恢复失败关闭；纯阅读和普通内容编辑
  按敏感动作枚举表执行。
- 错误、日志、审计 payload 与诊断信息不得包含密钥、Cookie、DSN 或原始敏感内容。

## 7. M0 统一验收

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
scripts/test-linux.sh
```

macOS 可作为快速回路；symlink、magic link、竞态、hardlink 与文件类型语义必须以 Linux
结果为准。CI 或 Docker 未实际运行时，只能记录“待验证”，不得声称 M0 已通过。
