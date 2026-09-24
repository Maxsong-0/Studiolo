# Studiolo

自托管的云端共享大脑与权限中枢：以 Markdown 保存内容，逐步建立面向 AI 的受控接口和协作能力，以及人的认证、审批与编辑入口。

- **产品需求与决策**：不在本仓库。权威档案在 Obsidian vault 的 `项目/Studiolo/`
  （INDEX 仪表盘、PRD 分册、ADR、TASKS 协调板、日志）。
- **本仓库**：实现代码、部署清单、面向实现者的技术方案（`docs/`）。
- **本页口径**：每条状态只分四档——**已在 main 且线上运行 / 已在 main 未部署 / 只在开发分支 / 未实现或未验收**，
  并给出可核对的依据。各切片的测试数字与验收证据留在 `docs/` 对应文件，本页不复制、不汇总。

## 本开发分支增量（2026-09-20，M5–M6）

T-088～T-091 的黑板、批量写回、Runner/Gateway、监督界面和 Demo 已实现并完成本地契约验证。
工作树为 `codex/m5-m6-blackboard-runner`，尚未提交、合并或部署。用户确认先完成代码与本地验收，
当前执行方向按用户澄清改为调用本机已登录的 Codex / Claude Code，仅使用订阅额度。新增本机连接程序、MCP/Channel 入口和黑板派发面板；安装与分项验收见 [本机客户端派发](docs/m6/local-client-dispatch.md)。服务器隔离路线仅保留既有证据，API 入口已停用；历史记录见 [实现与验收记录](docs/m6/implementation-acceptance.md)。
以下各段保留其原日期的历史口径，不作为本轮对线上状态的重新核验。

## 历史状态（2026-09-17）

参考点：`origin/main` = `297f126`（2026-09-17 合并 PR #5），本收尾分支
`claude/m0-3-wrap-up-a8db6d` = `b97475b`。**本地 `main` 分支引用停在 `41fa1b2` 未快进，
别拿它当权威**；用 `git fetch origin main` 后看 `origin/main`。本分支相对 `origin/main`
**领先 2、落后 2**（领先 `7770ac3`、`b97475b`；落后 `5854f91`、`297f126`），还没有对齐。
**M1、M2、M3 三个里程碑都没有完成，也都没有整体验收。**当前收尾工作为 T-047（回归与合并）、
T-048（备份 / 告警 / OwnedRun / Bridge 执行器）与 T-063（local-server 运行包），都在进行中。
逐工作域的细账在 [M2、M3 整体收尾台账](docs/m2/m2-m3-closeout.md)，但要注意**它是
2026-09-12 T-046 当时的快照，没有覆盖 T-047～T-063**；那三项的当前事实以本页与
各自文档为准。

### 已在 main 且线上实际运行

| 范围 | 事实 | 说明 |
|---|---|---|
| M0 基线 | Rust 库与 `M0Workbench`：真实文件系统 VaultBroker、SQLite TaskEngine、隔离 Stub Runner，以及索引、Gateway、审计的契约实现 | 历史证据见 [M0 验证记录](docs/contracts/verify-log.md)，不作为 M1/M2/M3 的验收记录 |
| M1 只读工作台 | T-006 总览 / T-007 搜索导航 / T-010 文件树 / T-011 界面重设计已实现，T-035 界面发布已合并 main `49a9f1a` 并由用户手动切换服务器 4319 网页服务 | 范围与验证状态见 [M1 实现说明](docs/m1/README.md)、[发布记录](docs/m1/ui-release.md)；**完整 M1 未完成**，公网登录后的交互验收待用户配合 |
| M3 供给面 | T-033/T-034 `studiolo-agent` 常驻服务器，一个 URL 供给冷启动路牌、白名单只读内容与读取审计 | 契约见 [供给面与 MCP 外壳](docs/contracts/agent-supply.md)，部署记录见 [服务器常驻](docs/m3/server-agent.md) |
| M3 资料同步 | T-036 Mac 每 60 秒发布白名单完整副本，MCP 四工具同代读取并提示过期 / 漂移；系统级服务以 `maxsong` 运行 | [新鲜度契约](docs/m3/replica-freshness.md)、[系统级运维](docs/m3/replica-operations.md) |
| M3 读取供给 | T-037 HTTP 只读文本入口、T-038 客户端投影 CLI、T-039 持久派生索引随 T-040 合并上线并通过公网复验 | [HTTP 文本供给](docs/m3/http-text.md)、[客户端投影](docs/m3/client-projection.md)、[持久派生索引](docs/m3/persistent-index.md)、[发布记录](docs/m3/read-supply-release.md) |

M1 的域名入口（T-009）Access、独立鉴权服务与 `studiolo.maxsong.me` 路由已部署生效，
**真实邮箱登录后的公网功能验收仍未完成**，见 [Cloudflare 域名部署](docs/m1/cloudflare-preview.md)。

### 已合并 origin/main，但**没有部署、没有整体验收**

2026-09-17 的 PR #5 把 T-041～T-046 的成果与 CI 修复合进了 `origin/main`（`297f126`）。
**合并不等于验收，更不等于上线**：服务器上跑的仍然是 T-040 的 M3 读取供给版本，
下面这些分片没有一个进入过生产。本轮未核对该次 main CI 的运行结果。

| 分片 | 内容 | 文档 |
|---|---|---|
| T-041 | 项目运作入口：显式绑定本地仓库、目录职责与双向目录漂移 | [项目运作图首片](docs/m3/project-operations.md) |
| T-042 | 有来源的模块关系、声明依赖 / 数据流与逐步设计路径 | [架构关系与任务路径](docs/m3/architecture-flow.md) |
| T-043 | 显式启用 Cargo 清单读取，派生成员包依赖并保留种类 / 条件 / 来源行 | [Cargo 清单依赖](docs/m3/cargo-dependencies.md) |
| T-044 | 显式 Rust 文件白名单派生「文件 → 词法上下文 → 调用语法位置」，目标未解析 | [Rust 调用位置证据](docs/m3/rust-call-sites.md) |
| T-045 | 同文件自由函数的受限词法关联，调用到定义并反查全部引用 | [函数词法关联](docs/m3/rust-call-links.md) |
| T-046 | MCP 会话与凭据边界、图 2 生产仓库批准、人侧正文 / Rename / 删除、流式附件、Bridge 规划与持久状态 | [收尾台账](docs/m2/m2-m3-closeout.md) 与 `docs/m2/` 各契约 |

T-041～T-045 曾在各自的 `codex/m3-*` 分支上开发验收，提交
`94801ef`、`7fe714c`、`c1c2171`、`82a620d`、`4f6f348` 与保全 T-046 未提交成果的 `ec19093`
**都已经是 `origin/main` 的祖先**（2026-09-17 用 `git merge-base --is-ancestor` 逐个核实）。
仍**没有进过 main** 的只有本分支领先的两个提交：`7770ac3`（接线 backup / observability /
sync-transport 三个新 crate 骨架）与 `b97475b`（本地那份 CI 磁盘修复；main 上是另一个
sha `5854f91`）。

### 只在本收尾分支，未合并、未部署、未验收

| 分片 | 内容 | 文档 |
|---|---|---|
| T-048 | 一致 checkpoint 与 BackupWorker、健康 / 配额 / 告警、SyncTransportOwner、MirrorCAS 与 Bridge 执行器 | 实施中，见下方「未实现或未验收」 |
| T-063 | local-server 运行包内容、本机出包与自检、服务器安装与回滚 | [运行包与部署脚本](docs/m3/release-packaging.md)；本机 / 容器内自检已过，**真实服务器部署与公网复验未做** |

### M4 策略平面：只在开发分支，未合并、未部署（2026-09-17 T-069）

| 分片 | 内容 | 文档 |
|---|---|---|
| T-069 | 规则母本／skills／能力集升入策略平面：`policy.sqlite3`、desired → 校验 → diff（扩大项标红）→ L3 One-time Code → effective、版本与回滚、实例／类型／全局三级吊销；接入方每次调用复核、供给视图＝部署上限∩能力集；人侧端点与「策略平面」视图 | [M4 说明](docs/m4/README.md)、[策略平面契约](docs/contracts/policy-plane.md)；**线上未启用**：agent 未给 `--policy`，人侧因子模式未部署 |

### 明确未实现或未验收（不得据本仓库宣称具备）

| 能力 | 当前事实 |
|---|---|
| 一致 checkpoint 与独立 BackupWorker | `crates/backup` 在 T-048 实施中，未验收、未合并、未在真实环境跑过；边界见 [checkpoint 与恢复契约](docs/m2/checkpoint-recovery-contract.md) |
| NAS 干净机恢复 | **未进行**。无 DSM 登录与专用备份目录，未做新机恢复演练；接入信息状态见 [接入信息](docs/m2/connection-setup.md) |
| 健康页与带外告警的生产启用 | `crates/observability` 与 `local-server` 健康模块在 T-048 实施中；生产未启用，告警邮箱已确认但**发送端与实际送达未验收** |
| 真实 OwnedRun | `crates/sync-transport` 在没有受控 Linux 隔离能力与管理员预置 launcher 特权时**一律返回 `UnsupportedContainment`**，绝不降级为 PID/PGID 观察冒充隔离；真实隔离运行未验收 |
| MirrorCAS 与 Bridge 执行器的生产接线 | `crates/sync-bridge` 的规划器与执行器是纯顺序核，不开文件、不连数据库、不发网络；**未接入真实同步，生产未接线** |
| 真实客户端安装 | T-038 的 CLI 已发布，但**没有安装到任何真实 Claude / Codex 工作区**，既有客户端配置未修改 |
| 公网发布验收 | 线上仍是 T-040 的 M3 读取供给版本；**本轮收尾版本未打包、未发布、未做公网验收** |
| 实际两周 headless 试跑 | **未进行**，也没有任何等价的长期运行证据 |
| 策略平面线上启用 | `crates/policy` 与两端接线已实现并本地验收；服务器 `studiolo-agent` 仍是 M3 兼容模式（无能力集、无吊销表），真实 One-time Code 发布未在生产验证 |

## 运行本机只读示例

在仓库根目录运行：

```bash
npm --prefix web ci --ignore-scripts
npm --prefix web run build
cargo run --locked -p studiolo-local-server -- --vault examples/m1-vault --assets web/dist --label 示例库 --port 4317
```

然后打开 [http://127.0.0.1:4317/](http://127.0.0.1:4317/)。入口固定使用 IPv4 loopback，
不要替换为 `localhost`；按 Ctrl-C 停止。完整参数（会话模式、canonical 只读 / 写者、
因子 Unix 入口、图 2 的仓库绑定与 Rust 白名单）以
`studiolo-local-server --help` 的实际输出为准，本页不复制那份清单。

上面三条命令 2026-09-17 在本收尾分支（`web/dist` 为既有构建产物）实跑过：服务器正常启动并打印
`Studiolo：http://127.0.0.1:4317（本机只读；按 Ctrl-C 停止）`，`/`、`/api/workbench`、
`/api/search`、`/api/document`、`/api/tree` 均返回 200，旧代次返回 409，`Host: localhost:4317`
被拒为 403。这是**本机示例入口可用**的证据，不是 M1 里程碑、公网入口或任何发布的验收证据。

`examples/m1-vault` 是纯虚构示例库。应用不会自动读取或复制真实 Vault，不修改源文件、
不创建 Vault 身份、数据库或缓存。查看真实内容时，只能由用户明确选择一个**只读副本**，
再将其路径传给 `--vault`；本仓库没有默认的真实笔记路径。

`examples/` 下另有四个虚构示例库（`m3-project-operations`、`m3-cargo-dependencies`、
`m3-rust-call-sites`、`m3-rust-call-links`），配合 `--project-repository` 等开关演示
T-041～T-045 的图 2 能力；运行命令写在**各自子目录**的 `README.md` 里（`examples/`
根目录没有汇总 README）。

服务器示例部署与 SSH 访问方式见 [服务器运行说明](docs/m1/server-preview.md)；
服务器运行不等于开放网络 HTTP，仍保持回环监听与虚构示例库范围。

## 结构

```text
Cargo.toml           # workspace：22 个成员、统一依赖版本、统一 lints
rust-toolchain.toml  # 1.97.1 + rustfmt + clippy
crates/
  core/              # studiolo-core：跨模块共享值类型（ID、Revision、Fence、权限集合、分级）
  vault-broker/      # canonical Vault 唯一写者：路径约束、CAS、历史、原子落盘
  vault-index/       # 解析、就近 wikilink、反链、标题树、frontmatter 归一
  control-db/        # 运行平面 SQLite WAL：schema、迁移、单写者连接
  audit-ledger/      # 权威审计唯一写入口，事件幂等与完整性链
  auth/              # M2 Access、独立持久会话、TOTP / 恢复码与固定槽因子
  auth-admin/        # 可信本地登记 CLI：固定秘密环境、隐藏 TTY 与明确迁移
  host-config/       # 管理 CLI 与服务器共用的私有非秘密配置 / 目录边界
  process-safety/    # 秘密驻留面一次性收紧（核心转储 / 进程检视 / 换出 / 环境槽）：只依赖 rustix，四个持秘密程序共用（T-065）
  sync-bridge/       # M2 双向 Bridge：三方规划、私有持久状态与 MirrorCAS 执行顺序核（未接真实同步）
  sync-transport/    # M2 SyncTransportOwner：固定 link、一次一个 OwnedRun；无受控隔离即拒绝
  backup/            # M2 一致 checkpoint、独立 BackupWorker 与新目录恢复（T-048 实施中）
  observability/     # M2 健康聚合、磁盘 / 同步配额、带外告警与死人开关（T-048 实施中）
  runner-adapter/    # Adapter Interface 与 M0 Stub；真实 CLI Adapter 属于后续里程碑
  gateway/           # capability 校验后的工具调用与模型中继 Seam
  task-engine/       # 任务命令、claim/attempt/fence、取消、projection outbox、对账
  studiolo/          # 组合根与集成测试（最小垂直链、reconcile、架构写入口）
  workbench/         # M1 纯派生视图、内存搜索、文档导航与有界文件树
  local-server/      # M1 只读查询；M2 显式会话、canonical 读写、人侧编辑 / 文件操作、健康与 Unix 因子 HTTP
  supply/            # M3 供给面内核：白名单只读根、冷启动路牌与 o200k 预算、类型 / 实例身份、读取审计索引
  policy/            # M4 策略平面：policy.sqlite3、三类母本 desired→校验→diff→L3 发布→effective、版本 / 回滚、三级吊销（T-069）
  agent-server/      # M3 `studiolo-agent`：rmcp Streamable HTTP 主外壳（仅 loopback）与 admin 子命令；M4 起 `--policy` 复核能力集与吊销
docs/                # M0 契约与 M1/M2/M3 分片实现、运行与验收说明
scripts/             # test-linux.sh（同步到 CI 服务器原生跑与 CI 相同的检查）、打包与 Python 管线检查
.github/workflows/   # CI（Linux x86_64，action 按 SHA 钉死）：Rust 作业 + 前端作业
web/                 # Vite + TypeScript + React 工作台：只读视图、M2 会话入口与人侧 Markdown 编辑器（未发布）
examples/            # 纯虚构示例库（M1 只读工作台与 M3 图 2 四组），不含真实用户笔记
deploy/              # agent/：M3 服务器单元与升级脚本；preview/：M1 示例的用户服务模板
```

依赖方向无环：`core` 不依赖任何成员；M0 的 `task-engine` 面向各 Module Interface 与
Control DB 编排，具体 Adapter 在 `studiolo` 组合根装配；`gateway` 不依赖 `task-engine`。
M1 的 `workbench` 只消费内存文档与 M0 索引；`local-server` 按启动参数分别装配只读派生视图、
会话认证、canonical 读取或写者。M2 `auth` 独占认证存储，不持有 ControlDb 或 Vault 能力；
`host-config` 共用私有配置检查。`backup` 不持有任何 owner 的数据库连接，只消费封存票据
指向的只读传输源；`sync-bridge` 纯规划 / 顺序核不做 IO；除 `vault-broker` 外的真实文件 IO
都必须收敛到架构守卫登记的固定文件（见 `crates/studiolo/tests/architecture*`）。

生产构建显式使用 `--features studiolo-auth/production`；该特征与虚构身份测试夹具互斥。
不要用示例路径或示例秘密替代经批准的生产配置。

## 开发

**所有测试与编译检查一律不在本机跑**（2026-09-23 用户硬约束）：`cargo test`／`fmt`／`clippy`／`build`、Python 契约、
前端检查、真实管线都经 `scripts/test-linux.sh <命令>`（在服务器上原生跑，可带参数跑任意定向命令）或 CI。
下面列出的命令是 CI 执行的内容，供对照与在服务器上复现，不是让你在本机跑。

下面这组命令覆盖 CI `rust` 作业里的编译类检查，**不是它的全集**（权威始终是
`.github/workflows/ci.yml`，缺的部分列在本节末尾）：

本机若装了 Homebrew Rust（2026-09-22 为 1.98.1），`/opt/homebrew/bin/cargo` 排在 PATH 前面时
不认 `rust-toolchain.toml`，clippy 结果会与 CI 不一致。先把 rustup 的钉死版本放到最前：

```bash
export PATH="$HOME/.rustup/toolchains/1.97.1-$(rustc -vV | sed -n 's/^host: //p')/bin:$PATH"
```

```bash
cargo --locked fmt --all -- --check
cargo --locked clippy --workspace --all-targets -- -D warnings
cargo --locked test --workspace
cargo --locked test -p studiolo-vault-broker --test linux_path_security   # 只在 Linux 有真实语义
cargo --locked build -p studiolo-local-server --features studiolo-auth/production
cargo --locked build -p studiolo-auth-admin --features production
```

前端（CI 原生运行，`setup-node` 钉死 Node 22.17.0，工作目录 `web`）：

```bash
npm --prefix web ci --ignore-scripts && npm --prefix web run build
npm --prefix web test && npm --prefix web run lint && npm --prefix web run format
```

不连后端看界面：`STUDIOLO_FIXTURES=1 npm --prefix web run dev` 用 `web/dev/` 的假数据应答全部 `/api/*`
（写操作只改内存，重启复原）。只在 `vite serve` 下生效，生产构建里不出现（T-130）。

双向 Bridge 平面默认关闭：只有 `--sync-transport </绝对路径/sync-approval.json>` 且私有配置带 `sync_scope` 才装配，
四项私有存储先用 `studiolo-maintain sync-init`（任一已存在即拒绝）显式建立，已不再排除的文件夹在镜像视图里的残留
用 `studiolo-maintain sync-purge-residue --folder <名>` 清除（须先停 `studiolo-human`）；设计见 `docs/m2/sync-plane-design.md`。

上面那组之外，CI `rust` 作业还会做四件事（2026-09-22 按 `ci.yml` 逐条核对）：

- 对**八个显式列名**的脚本跑 `bash -n`：`scripts/` 下的 `package-agent.sh`、
  `run-agent-tests.sh`、`sync-agent-vault.sh`、`package-preview.sh`、`run-preview-tests.sh`，
  以及 `deploy/agent/install-t036-system.sh`、`deploy/agent/upgrade-read-supply.sh.in`、
  `deploy/agent/enable-m4-policy.sh.in`。
  **这不是 `bash -n` 全部部署脚本**：`deploy/preview/install-runtime-package.sh` 不在清单里。
- 跑**逐条列名**的 `scripts/test-*.py`（不是 `test-*.py` 通配；2026-09-23 为 25 个，其中
  `test-human-offline.py` 以 `python3 -I -B` 跑），另跑 `deploy/blackboard/test_{assembly,freeze,activation}.py`
  与 `scripts/tests/test_runner_launch.py`，再在 `umask 002` 下复跑副本同步三份（T-079）。
- `cargo --locked build -p studiolo-agent-server` 后跑
  `scripts/check-replica-pipeline.py`、`scripts/check-client-projection-pipeline.py` 两条真实管线。
- 生产 `studiolo-auth-admin` 构建之后再跑
  `python3 crates/auth-admin/tests/support/cli_process.py target/debug/studiolo-auth-admin`。

在 Linux 服务器上原生跑 Rust 那一半（把工作树同步到 `githubci` 后在那里跑；2026-09-19 起不再用 Docker）：

```bash
scripts/test-linux.sh   # fmt / clippy / workspace 测试 / 路径安全 / 两个生产构建 / 登记工具进程检查
```

`scripts/test-linux.sh` **不包含**上面的 `bash -n`、逐条列名的 Python 契约脚本、两条管线检查，
也不含前端作业；要复跑那些得自己按 `ci.yml` 逐条执行。

## 纪律

- 真实密钥只经环境变量或 secret mount 注入，永不入库。
- 提交信息中文，带 Conventional Commits 前缀。
- CI 在 Linux 上跑路径安全测试；macOS 只作快速回路。
- 进程内只有 VaultBroker 可写 canonical Vault；架构测试枚举全部写入口。
- 文档不得冒称完成度：没有实跑证据的一律写「未验证 / 未验收」，不用「构建成功」代替发布证据。
