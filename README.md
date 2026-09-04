# Studiolo

自托管的知识与项目工作台：Markdown 知识库、项目管理面板，以及受控的多 agent 闭环。

- **产品需求与决策**：不在本仓库。权威档案在 Obsidian vault 的 `项目/Studiolo/`
  （INDEX 仪表盘、PRD 分册、ADR、TASKS 协调板、日志）。
- **本仓库**：实现代码、部署清单、面向实现者的技术方案（`docs/`）。
- 状态：M0（接口与索引底座）实现与本地验收完成。macOS 102 项测试、Linux 110 项测试通过，格式、Clippy 与文档构建通过；完整证据见 [验证记录](docs/contracts/verify-log.md)。

M0 提供 Rust 库与 `M0Workbench` 组合入口：真实文件系统 VaultBroker、SQLite TaskEngine、
隔离 Stub Runner，以及索引、Gateway 和审计的契约实现。当前没有网页或常驻服务入口；
审计仍使用内存 Adapter。远端 CI 的运行结果以 GitHub Actions 为准。

## 结构

```text
Cargo.toml           # workspace：成员、统一依赖版本、统一 lints
rust-toolchain.toml  # 1.97.1 + rustfmt + clippy
crates/
  core/              # studiolo-core：跨模块共享值类型（ID、Revision、Fence、权限集合、分级）
  vault-broker/      # canonical Vault 唯一写者：路径约束、CAS、历史、原子落盘
  vault-index/       # 解析、就近 wikilink、反链、标题树、frontmatter 归一
  control-db/        # 运行平面 SQLite WAL：schema、迁移、单写者连接
  audit-ledger/      # 权威审计唯一写入口，事件幂等与完整性链
  runner-adapter/    # Adapter Interface 与 M0 Stub；真实 CLI Adapter 属于后续里程碑
  gateway/           # capability 校验后的工具调用与模型中继 Seam
  task-engine/       # 任务命令、claim/attempt/fence、取消、projection outbox、对账
  studiolo/          # 组合根与集成测试（最小垂直链、reconcile、架构写入口）
docs/                # 面向实现者的契约：Module Interface、Control DB schema
scripts/             # test-linux.sh：Docker 内跑与 CI 相同的 fmt / clippy / test
.github/workflows/   # CI（Linux x86_64，action 按 SHA 钉死）
web/                 # （M1）Vite + TypeScript + CodeMirror 6 前端
deploy/              # （M2）Docker Compose、cloudflared、restic、恢复脚本
```

依赖方向无环：`core` 不依赖任何成员；各深模块公开自己的对象安全接口，`task-engine` 只面向这些接口与 Control DB 编排，具体 Adapter 在 `studiolo` 组合根装配；`gateway` 不依赖 `task-engine`。

## 开发

```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked
scripts/test-linux.sh   # 同一套在 Linux 容器内跑（路径安全负向测试只在 Linux 有真实语义）
```

## 纪律

- 真实密钥只经环境变量或 secret mount 注入，永不入库。
- 提交信息中文，带 Conventional Commits 前缀。
- CI 在 Linux 上跑路径安全测试；macOS 只作快速回路。
- 进程内只有 VaultBroker 可写 canonical Vault；架构测试枚举全部写入口。
