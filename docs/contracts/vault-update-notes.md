# M0 权威档案回写对照（已完成）

> 2026-09-04：主线已完成真实 vault 回写，文档线只读核对了技术栈、写路径图、敏感动作表
> 和归档计划。本页保留代码与权威档案之间的对照依据；权威目录为 `项目/Studiolo/`，
> M0 计划已归档到 `04-协调/归档/2026-09-04-M0-实施计划.md`。

## 1. 十一项退出标准的证据位置

| M0 计划项 | 仓库证据 | 权威档案收尾 |
|---|---|---|
| 1 六个 Module Interface 与评审 | `docs/contracts/`、各 crate 公开 trait 与契约测试 | M0 计划登记最终设计与评审结论 |
| 2 canonical Vault 唯一写入口 | `crates/studiolo/tests/architecture.rs` | 写路径图与当前依赖/方法一致 |
| 3 同 DB 重启 reconcile | TaskEngine 重启契约测试、真实文件系统集成测试 | 记录 actual pass 与未覆盖范围 |
| 4 Control DB 与 Vault UUID | ControlDb migration/绑定测试、Broker revision 测试 | 技术栈/目录结构同步 schema 版本 |
| 5 最小垂直链 | `crates/studiolo/tests/` 中真实 Fs Broker + SQLite + Stub 测试 | 日志记录 task→manifest→投影结果 |
| 6 Linux 路径负向 | `crates/vault-broker/tests/linux_path_security.rs` | 只根据实际 Linux 运行记录通过 |
| 7 toolchain 与 crate 锁定 | `rust-toolchain.toml`、`Cargo.toml`、`Cargo.lock` | 技术栈逐项对应最终依赖 |
| 8 Linux CI | `.github/workflows/ci.yml`、`scripts/test-linux.sh` | 使用方式写同一组命令；推送后才能产生 CI 运行证据 |
| 9 敏感动作枚举 | 既有 `01-框架/敏感动作枚举表.md` | 保留 M2–M4 端点范围；区分 M0 审计 action 与未来端点事件候选 |
| 10 写路径架构图 | `00-overview.md`、`vertical-chain.md`、架构测试 | 按下节修正旧候选设计 |
| 11 中英文别名 | `vault-index.md`、常量与对照测试 | 现权威表保留，不做全库回填 |

## 2. `01-框架/技术栈.md`

已按最终 Cargo 文件回写，区分 2026-09-02 候选依赖与当前已用能力：

- toolchain 固定 1.97.1；edition 2024、resolver 3。`rust-version` 是最低 Rust 要求，
  不取代固定 toolchain。
- `serde-saphyr` 为读侧 YAML 与投影写后校验；写侧仍为顶层标量行级补丁。
- `tempfile` 是 Stub Runner 创建私有 attempt 临时目录的正常依赖，不能再标成“仅测试”。
- `jiff` 在 AuditLedger 的服务端记录时间实际使用；任务 lease 当前以显式 Unix 毫秒 DTO
  传递，不宣称已实现 scheduler/retention。
- `hmac` 原为 capability 签名候选，本轮已从 workspace 移除；M0 capability 是进程内 DTO，
  不具备签名。技术栈应同步删除“已锁定使用”条目。
- Control DB 单写者为 TaskEngine；`ControlDb` 方法封装事务不变量，不暴露底层连接。
- 审计当前是带幂等与前向链的内存 Adapter，不宣称已有审计文件耐久性。
- Linux 路径实现显式使用 openat2，内核不支持时失败关闭；当前不实现 Linux cap-std 回退。
  不可用历史研究说明代替本轮证据，也不对任意带外外部写进程声称严格线性化替换 CAS。

其他已锁版本已与 `Cargo.lock` 对照；后续升级继续在当日日志留痕。CI 第三方 action SHA、
toolchain 与 Linux 容器镜像以最终脚本为准。

## 3. `01-框架/写路径架构图.md`

已保留未来 Web/API、bridge、Gateway 提升等路线，并显式标明非 M0；主要对齐点如下：

1. 六 Module 仍为 VaultBroker、VaultIndex、TaskEngine、Gateway、AuditLedger、Runner Adapter。
   `core/control-db` 是支撑 crate，`studiolo` 是组合根。
2. Broker Seam 实际由 `VaultReader`、`VaultSnapshotter`、`SnapshotReader`、`VaultWriter`
   组成。`snapshot` 保存不可变输入到 data root，不写 canonical 文件。
3. VaultIndex M0 由调用方传入 Markdown 批次做 `rebuild`；删除图中作为已实现能力的
   `changes(cursor)` 连线，改为 M1 增量索引候选。
4. Control DB 行改为“TaskEngine 独占 ControlDb；ControlDb 在事务内强制
   transition/fence/outbox 不变量，其他 Module 不执行写 SQL”。
5. M0 Broker 不持有 AuditLedger。TaskEngine 先写 intent，再调用 Broker 提交；Gateway
   自己经 AuditLedger 记录调用。Broker→AuditLedger 若保留，应标后续 Adapter 设计。
6. Runner 成功顺序是 `report`→T1 封存计划→intent→Broker artifact/manifest→T2（终态、
   projection outbox、terminal audit outbox）→outcome→ack。outcome 失败可从 outbox 补记，
   不重新运行 Runner，不重复提交 artifact。
7. manifest 唯一路径为 `sources/attempts/<task-id>/<attempt-id>/manifest.json`。已存在
   manifest 必须验证身份/摘要，不能只凭文件存在判成功；无 manifest 的 orphan 不算成功。
8. task 页面为 `ops/tasks/<task-id>.md`。投影只改
   `status/active_attempt/last_attempt/input_snapshot`；保留未知键、注释、顺序和正文。
9. M0 Stub 没有重启后的存活进程。reconcile 收敛旧 Running/CancelRequested，并前滚持久
   Committing；真实 CLI 的隔离/回收时限仍属后续 Runner 验收。

## 4. `01-框架/敏感动作枚举表.md`

M0 不实现列出的 HTTP 端点。已保留现有端点动作与等级，并补充当前 M0 的 action 为
`task_engine.report`、`gateway.tool_call`、`gateway.model_relay` 及实现中的拒绝/复查路径；
这些不是完整端点事件枚举。端点能力上线前，再把候选审计事件与真实动作一一绑定。

## 5. 收尾状态与归档

最终验证完成后，主线已按 macOS 102 项、Linux 110 项及格式/Clippy/文档构建通过结果同步：

- `01-框架/使用方式.md`：固定版本命令、工作树位置、Linux 快速回路与未实现产品入口。
- `01-框架/目录结构.md`：九个 crate、契约、CI/脚本、测试；不加入尚不存在的服务。
- `02-日志/2026-09-04.md`：实现摘要、纠正的契约、完整验证命令/环境/结果、验收时尚未提交推送的快照；后续 Git 操作另行追加。
- `00-总览/INDEX.md`、`04-协调/TASKS.md`、`05-版本/roadmap.md`：T-004/T-005 与 M0 完成状态
  一致；Git 提交与推送按用户后续明确指令登记。
- 原 `04-协调/M0-实施计划.md` 已逐项登记证据，按项目规则同文件改名归档到
  `04-协调/归档/2026-09-04-M0-实施计划.md`，当前引用同步指向归档。

本轮没有创建提交或推送；Git 提交仍由用户明确决定。
