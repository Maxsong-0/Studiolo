# Studiolo

自托管的知识与项目工作台：Markdown 知识库、项目管理面板，以及受控的多 agent 闭环。

- **产品需求与决策**：不在本仓库。权威档案在 Obsidian vault 的 `项目/Studiolo/`
  （INDEX 仪表盘、PRD 分册、ADR、TASKS 协调板、日志）。
- **本仓库**：实现代码、部署清单、面向实现者的技术方案（`docs/`）。
- 状态：M0 之前，尚无实现代码。本次提交仅用于占位与建立工程骨架。

## 结构（规划）

```text
crates/
  studiolo/        # 占位 crate，用于 crates.io 名称保留；后续为二进制入口
  vault-broker/    # 路径约束、CAS、历史、原子落盘、审计
  task-engine/     # 任务命令、attempt、fencing、取消、对账
  gateway/         # web.fetch、db.*、模型中继、策略求交
  audit-ledger/    # 权威审计唯一写入口，事件幂等与完整性链
  runner-adapter/  # Adapter Interface 与 stub / claude-code / codex 实现
web/               # Vite + TypeScript + CodeMirror 6 前端
deploy/            # Docker Compose、cloudflared、restic、恢复脚本
```

## 纪律

- 真实密钥只经环境变量或 secret mount 注入，永不入库。
- 提交信息中文，带 Conventional Commits 前缀。
- CI 在 Linux 上跑路径安全测试；macOS 只作快速回路。
