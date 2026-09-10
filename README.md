# Studiolo

自托管的云端共享大脑与权限中枢：以 Markdown 保存内容，逐步建立面向 AI 的受控接口和协作能力，以及人的认证、审批与编辑入口。

- **产品需求与决策**：不在本仓库。权威档案在 Obsidian vault 的 `项目/Studiolo/`
  （INDEX 仪表盘、PRD 分册、ADR、TASKS 协调板、日志）。
- **本仓库**：实现代码、部署清单、面向实现者的技术方案（`docs/`）。
- 状态：**T-006 总览、T-007 搜索 / 导航、T-010 文件树 / 附件信息及 T-011 界面重设计已实现，完整 M1 尚未完成**。当前提供项目总览、待我动作、原文行号、手动刷新、中文搜索、快速切换、大纲 / 双链，以及包含空目录与纯附件目录的文件树；范围和验证状态见 [M1 实现说明](docs/m1/README.md)。
- **M3 首片已合并 main 并常驻服务器（2026-09-07，T-033/T-034）**：`studiolo-agent` 以一个 URL 向 AI 接入方供给冷启动路牌（≤2000 token 二元）、白名单内只读内容与读取审计；契约见 [供给面与 MCP 外壳](docs/contracts/agent-supply.md)。公网入口 `https://studiolo-agent.maxsong.me/mcp`（专用隧道 → 服务器回环 4400，应用内 bearer 门禁），部署与验收记录见 [服务器常驻](docs/m3/server-agent.md)。真实库验收：路牌 931 token；真实 Claude Code 经公网握手成功。
- **M3 资料同步与新鲜度已合并（2026-09-10，T-036，main `66dc7e3`，CI 通过）**：白名单稳定捕获、完整批次发布、失败保留旧副本、MCP 四工具同代读取与过期/漂移提示；Mac 每 60 秒自动同步已启用。服务器原生 302 项通过；托管 MCP 的用户级 systemd 切换因 UID 映射被拒绝并已恢复旧版，须按 [系统级启用步骤](docs/m3/replica-operations.md) 完成换版。详见 [验收记录](docs/m3/replica-freshness.md)；ADR-015 客户端配置推模式仍待实现。
- **当前优先 M2**：认证前后端、canonical单写者/永久配对/未决幂等及T-029新main整合已验收；当前macOS747/Linux758和两端生产通过，前端沿用T-025的125项。HEAD已d24e56a，继续新鲜度/CAS；编辑、同步、备份恢复和完整发布仍未完成，见[M2验证记录](docs/m2/README.md)与[发布缺口](docs/m2/release-readiness.md)。

M1 使用独立的只读 Broker Adapter、纯派生 Workbench Module 与本机网页入口，不装配 M0
写引擎。搜索、摘要、原文与导航共享同一不可变 generation，详见
[搜索与导航契约](docs/m1/search-navigation.md) 与 [文件树契约](docs/m1/file-tree.md)。
附件仅显示路径、后缀与大小，不读取内容。变更流、规则漂移、持久索引和 watcher 仍属后续范围；当前不提供编辑能力。

M0 基线仍保留 Rust 库与 `M0Workbench`：真实文件系统 VaultBroker、SQLite TaskEngine、
隔离 Stub Runner，以及索引、Gateway 和审计的契约实现；审计仍使用内存 Adapter。
M0 当时通过 macOS 102 项、Linux 110 项测试，以及格式、Clippy 和文档构建；历史证据见
[M0 验证记录](docs/contracts/verify-log.md)，不作为本轮 M1 的验收记录。

## 运行本机只读示例

在仓库根目录运行：

```bash
npm --prefix web ci --ignore-scripts
npm --prefix web run build
cargo run --locked -p studiolo-local-server -- --vault examples/m1-vault --assets web/dist --label 示例库 --port 4317
```

然后打开 [http://127.0.0.1:4317/](http://127.0.0.1:4317/)。入口固定使用 IPv4 loopback，
不要替换为 `localhost`；按 Ctrl-C 停止。

`examples/m1-vault` 是纯虚构示例库。应用不会自动读取或复制真实 Vault，不修改源文件、
不创建 Vault 身份、数据库或缓存。查看真实内容时，只能由用户明确选择一个**只读副本**，
再将其路径传给 `--vault`；本仓库没有默认的真实笔记路径。

T-007 当时通过 macOS 158 项、Linux 166 项 Rust 测试及格式 / Clippy 检查，前端 28 项测试、
lint、Prettier 与生产构建。本轮真实 HTTP 桌面与窄屏搜索/导航操作通过，控制台无警告或错误；
T-006 当时的测试与视觉验收保留为历史，详见 [M1 验收与差异记录](docs/m1/README.md)。当前分支
`codex/m1-readonly-workbench` 尚未提交推送，M1 远端 CI 尚未触发。

用户指定的服务器示例部署与 SSH 访问方式见 [服务器运行说明](docs/m1/server-preview.md)。
服务器运行不等于开放网络 HTTP，仍保持回环监听与虚构示例库范围。
后续 [Cloudflare 域名部署](docs/m1/cloudflare-preview.md) 的 Access、独立鉴权服务和
`studiolo.maxsong.me` 路由已生效；服务器原生测试通过，仍待真实邮箱登录及公网功能验收。
T-009 新版验证为 macOS 189 / 服务器原生 197 项 Rust、前端 28 项及 17 项 HTTP 检查。

T-010历史验证：macOS222/Linux aarch64 230项Rust、前端39项与生产构建、格式检查通过；
桌面与窄屏文件树操作通过。入口为「文档 → 文件树」。后续结果以M1/M2各检查点为准，当前成果尚未提交推送或更新服务器发布。

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
  auth/              # M2 Access、独立持久会话、TOTP/恢复码与固定槽因子
  auth-admin/        # 可信本地登记CLI：固定秘密环境、隐藏TTY与明确迁移
  host-config/       # 管理CLI与服务器共用的私有非秘密配置/目录边界
  runner-adapter/    # Adapter Interface 与 M0 Stub；真实 CLI Adapter 属于后续里程碑
  gateway/           # capability 校验后的工具调用与模型中继 Seam
  task-engine/       # 任务命令、claim/attempt/fence、取消、projection outbox、对账
  studiolo/          # 组合根与集成测试（最小垂直链、reconcile、架构写入口）
  workbench/         # M1 纯派生视图、内存搜索、文档导航与有界文件树
  local-server/      # M1只读查询；M2显式会话、canonical读取及Unix因子HTTP
  supply/            # M3 供给面内核：白名单只读根、冷启动路牌与 o200k 预算、类型/实例身份、读取审计索引
  agent-server/      # M3 `studiolo-agent`：rmcp Streamable HTTP 主外壳（仅 loopback）与 admin 子命令
docs/                # M0 契约与 M1/M2 分片实现和运行说明
scripts/             # test-linux.sh：Docker 内跑与 CI 相同的 fmt / clippy / test
.github/workflows/   # CI（Linux x86_64，action 按 SHA 钉死）
web/                 # Vite + TypeScript + React只读工作台与M2会话入口；尚无编辑器
examples/m1-vault/   # 纯虚构示例库，不含真实用户笔记
deploy/preview/      # M1 服务器示例的用户服务模板；完整 M2 部署设施尚未提供
```

依赖方向无环：`core` 不依赖任何成员；M0 的 `task-engine` 面向各 Module Interface 与
Control DB 编排，具体 Adapter 在 `studiolo` 组合根装配；`gateway` 不依赖 `task-engine`。
M1 的 `workbench` 只消费内存文档与 M0 索引，`local-server` 单独装配只读 Adapter 和派生视图。
M2 `auth` 独占认证存储，不持有ControlDb或Vault能力；`host-config`共用私有配置检查，
CLI和服务器分别装配可信登记与运行接口。会话、TOTP及恢复码已接入显式HTTP组合；当前进展见[M2开发状态](docs/m2/README.md)。
v2已通过显式迁移和安全历史回收契约，保留旧活跃凭证，防回收后旧秘密复用；v3预配置TOTP/会话近期认证也已通过双平台契约验收。
v4恢复码换发与真实内部因子替换已通过双平台契约验收，见[恢复契约](docs/contracts/auth-recovery.md)。
CLI及固定秘密启动已经实现并用虚构夹具验收；真实主人登记、手机配对、恢复码保管和HTTPS接入仍需实操，完整M2未合并部署。
已有[会话启动](docs/contracts/auth-startup.md)和[Unix因子模式](docs/contracts/auth-factor-http.md)契约；不要用示例路径/秘密替代经批准的生产配置。
生产构建显式使用 `--features studiolo-auth/production`；该特征与虚构身份测试夹具互斥。

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
