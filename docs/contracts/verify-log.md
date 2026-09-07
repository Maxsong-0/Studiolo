# M0 契约与实现验证记录

> 本页只记录实际执行过的验证。计划、代码存在或“应当通过”不等于通过；每次运行必须写
> 日期、环境、代码状态、完整命令和结论。worktree 中的未提交改动会变化，验证时需记录
> `git status --short` 与 HEAD。

## 0. 最终本地验收（2026-09-04）

M0 实现与本地验收完成。以下为主线在最后一轮代码冻结后的实际执行结果，
不是依据 workflow 文件或测试源码推断。验收当时远端 GitHub Actions 未触发，代码未提交、未推送；此节保留该时点快照。

代码位置：`.claude/worktrees/m0-phase-start-b1fa23`，分支
`claude/m0-phase-start-b1fa23`，HEAD `6774dac59d21405f3f80f2b0366e212819c691ae`。
验收时工作树为未提交 M0 改动：Cargo/README 已修改，各 Module、测试、docs/contracts、CI 与脚本
为本轮新增/修改；不声称 clean 或用 HEAD 代表全部已验证代码。

| 检查 | 环境与结果 |
|---|---|
| 格式与 Clippy | macOS arm64、Rust 1.97.1；全部 exit 0，无 warning |
| Workspace 测试 | macOS arm64；102 项通过，Linux 专属文件在此平台不编译执行 |
| Linux 完整快速回路 | Docker 29.3.1，`rust:1.97.1-bookworm`，Linux `7.0.12-linuxkit` aarch64；fmt、Clippy、110 项测试全部通过 |
| Linux 显式路径验收 | 同一脚本额外执行 8 项 Linux 路径负向测试，8/8 通过 |
| Rust 文档 | `cargo doc` exit 0 |
| 文档结构 | 13 份 contracts；相对 Markdown 链接 0 断链、0 尾随空白；`git diff --check` 通过 |
| 权威档案 | 主线已回写技术栈/使用方式/目录/图/敏感表/ADR 与协调状态；M0 计划已同文件归档，见 `vault-update-notes.md` |
| 远端 CI | 未运行；workflow 已配置 Linux x86_64，不能用本地 aarch64 结果替代远端运行证据 |

实际命令（工作目录均为上述 worktree）：

```bash
cargo fmt --all && cargo fmt --all -- --check && cargo clippy --workspace --all-targets --locked -- -D warnings && cargo test --workspace --locked --quiet
scripts/test-linux.sh
cargo doc --workspace --no-deps --locked
```

Linux 脚本直接运行，无 `STUDIOLO_*` 环境覆盖；脚本内执行固定 toolchain 的 fmt、Clippy、
完整 workspace test，再显式执行 `studiolo-vault-broker --test linux_path_security`。

macOS 102 项分布：studiolo unit 1、架构 7、真实垂直链/重启 3、AuditLedger 7、ControlDb 11、
core 10、Gateway 14、Runner 5、投影 6、TaskEngine 18、Broker 16、VaultIndex 4。Linux 在此
基础上增加 8 项路径测试，总计 110；显式第二次运行的 8 项不重复计入总数。

行为证据覆盖：Run 冻结输入与 NeedsReconfirm、四字段投影保留人写字节、取消竞态与
同 DB 重启、无/部分/完整 manifest 的前滚、T2 后 audit outbox 故障恢复、旧 receipt 重验、
Vault-bound snapshot/manifest 完整性、路径竞态与唯一写入口。真实网络/数据库/模型工具、
CLI Runner、审计文件耐久性、常驻服务与远端发布不属于本轮结果。

Linux Adapter 要求 openat2；不支持该调用时失败关闭，没有未验证的 Linux 路径回退。
本轮 Linux 证据来自上述支持 openat2 的内核，不声称已运行旧内核 ENOSYS 注入测试。

## 1. 2026-09-03 局部记录（不是最终验收）

2026-09-04 恢复工作时重新核验：`task-engine.md` 缺失，本轮已补齐；并新增
`vault-update-notes.md`。因此下表的历史 12/12 记录不能证明恢复时或最终工作树的文件状态。
本轮 TaskEngine/ControlDb 的输入冻结、取消、审计 outbox 与投影仍以最终全量验证为准。

| 检查 | 状态 | 证据/待办 |
|---|---|---|
| 12 份契约文件存在 | 已通过结构检查 | 2026-09-03：expected=12、actual=12、missing=0；只证明文件结构 |
| README/契约相对 Markdown 链接有效 | 已通过结构检查 | 2026-09-03：broken=0；不证明链接内容正确 |
| 权威范围映射 | 已完成文档审阅 | 对照 roadmap、M0 实施计划、PRD 入口覆盖表、ADR-001/003/005/007/008/010/012；不代表代码通过 |
| Rust Interface 与契约逐项一致 | 待实现完成后核对 | VaultIndex 与 Gateway 已按本轮裁决收敛文档；仍须核 Committing、manifest 路径、字段归属和全部错误 enum |
| VaultIndex 与权威别名表一致 | 局部验证通过 | 实现线 2026-09-03 10:27 PDT 跑该 crate 4/4 契约测试、clippy/fmt 均 exit 0；完整 workspace 仍未验证 |
| AuditLedger / Gateway M0 契约 | 局部验证通过 | 实现线 2026-09-03 10:28 PDT 跑 21/21 契约测试、clippy/fmt 均 exit 0；仅内存 Adapter |
| `cargo fmt --all -- --check` | 未运行 | 由实现/验证线在代码稳定后执行 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 未运行 | 同上 |
| `cargo test --workspace` | 未运行 | 同上 |
| `scripts/test-linux.sh` | 未运行 | Linux 路径语义的最终依据 |
| 写入口架构测试 | 未运行 | 必须结合 Cargo 依赖图与源码扫描 |
| 最小垂直链 | 未运行 | 需验证 manifest 最后写与 needs_review 投影 |
| 同 DB 重启 reconcile | 未运行 | spy Runner 必须证明 start 次数不增加 |
| T-005 三表与最终代码名一致 | 待核对 | 审计事件名仍以本仓库契约/实现为准 |

### 已执行：文档结构（2026-09-03，macOS arm64）

HEAD：`6774dac`；docs 状态：`M docs/README.md`、`?? docs/contracts/`。

```zsh
expected_files=(00-overview.md core-types.md vault-broker.md vault-index.md control-db.md task-engine.md runner-adapter.md gateway.md audit-ledger.md vertical-chain.md decisions.md verify-log.md)
for expected_file in "${expected_files[@]}"; do test -f "docs/contracts/$expected_file"; done
test "$(find docs/contracts -maxdepth 1 -type f -name '*.md' | wc -l | tr -d ' ')" -eq 12
for markdown_file in docs/README.md docs/contracts/*.md; do
  while IFS= read -r link_target; do
    link_base=${markdown_file:h}
    test -f "$link_base/$link_target"
  done < <(perl -ne 'while (/\]\(([^)#]+\.md)(?:#[^)]*)?\)/g) { print "$1\n" }' "$markdown_file")
done
if rg -n '[[:blank:]]+$' docs/README.md docs/contracts; then exit 1; fi
```

结果：12/12 文件存在；相对 `.md` 链接逐一按所在目录检查，0 个断链；0 行尾随空白。
未运行 Rust 构建、测试或 Linux 检查。

### 已执行：VaultIndex 局部验证（实现线报告）

时间/环境：2026-09-03 10:27:43 PDT；Darwin arm64；Rust/Cargo 1.97.1 Homebrew。
HEAD：`6774dac59d21405f3f80f2b0366e212819c691ae`；分支：`claude/m0-phase-start-b1fa23`；
`git status --short -- crates/vault-index` 为 `?? crates/vault-index/`，未声称全工作树 clean。

```bash
cargo fmt -p studiolo-vault-index && cargo test -p studiolo-vault-index && cargo clippy -p studiolo-vault-index --all-targets -- -D warnings && cargo fmt -p studiolo-vault-index -- --check
rg -n 'std::fs|File::create|OpenOptions|remove_file|remove_dir|rename\(|write_all|fs::write|tokio::fs' crates/vault-index
```

结果：组合命令 exit 0；integration 4/4（frontmatter 2、link 1、source profile 1），unit/doctest
均 0/0；clippy 无 warning；fmt check exit 0。只读写 API 扫描无命中。此记录来自实现线实际
运行，不等于完整 workspace 或独立复核，也不覆盖 Linux。

### 已执行：AuditLedger / Gateway 局部验证（实现线报告）

时间/环境：2026-09-03 10:28 PDT；aarch64-apple-darwin；Rust/Cargo 1.97.1。
HEAD：`6774dac59d21405f3f80f2b0366e212819c691ae`；分支：`claude/m0-phase-start-b1fa23`；
局部状态为 `?? crates/audit-ledger/`、`?? crates/gateway/`，未声称全工作树 clean。

```bash
cargo fmt -p studiolo-audit-ledger -p studiolo-gateway -- --check
cargo test -p studiolo-audit-ledger -p studiolo-gateway --locked
cargo clippy -p studiolo-audit-ledger -p studiolo-gateway --all-targets --locked -- -D warnings
```

三条命令均 exit 0；Audit integration 7/7、Gateway integration 14/14，合计 21 项；两 crate
unit/doctest 均 0；clippy 无 warning。此证据仅覆盖同步 Interface 与内存 Adapter，不覆盖
真实外部 I/O、完整 workspace、Linux 或独立复核。

## 2. 必跑命令

工作目录：仓库根目录。

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- --deny warnings
cargo test --workspace --locked
scripts/test-linux.sh
```

若 `scripts/test-linux.sh` 未运行或 Linux 专属用例被 skip，必须把路径安全结论写成未验证。
GitHub Actions 只有在用户决定推送后才产生证据，不得用 workflow 文件存在代替运行结果。

## 3. 行为验收清单

### 六 Module

- [x] 每个 Module 有可编译 Interface、结构化错误、生产/内存 Adapter 或明确的 M0 替身。
- [x] fake 通过公开 Seam 的契约测试，保留拒绝、幂等与故障注入，不是无条件成功。
- [x] Gateway 不依赖 TaskEngine；core 不依赖 workspace 成员。
- [x] VaultIndex 的主/BUG 日期 alias 常量覆盖权威别名表全部行；局部契约测试见上。

### VaultBroker

- [x] CAS、history、原子落盘、幂等、Vault UUID-bound revision。
- [x] manifest 固定路径、最后写、重复同内容成功、异内容拒绝。
- [x] Linux 用例覆盖 symlink/magic link、hardlink、FIFO/socket、编码分隔符、大小写/Unicode
  碰撞、文件/父目录竞态替换与并发 create-only；8/8 通过。

### Control DB / TaskEngine

- [x] 空库迁移、错 Vault、唯一 active attempt、单调 fence、v1 真补列升级。
- [x] 状态与 projection outbox 同 transaction；T2 同时保存 terminal audit outbox。
- [x] `running -> committing -> needs_review` 两阶段。
- [x] 崩溃矩阵使用同一 DB 文件；reconcile 只前滚，不调用 Runner start。
- [x] task 人写字段不被投影覆盖，直接改投影字段不产生授权。

### Runner / Gateway / Audit

- [x] Stub 输出仅在 attempt temp，且绑定 snapshot 输入。
- [x] 旧 fence、取消、迟到报告全部拒绝。
- [x] Gateway 内存 Adapter：默认拒绝、四层权限求交、call 幂等、intent-before-result。
- [x] Audit 内存 Adapter：event+phase 幂等、SHA-256 前向链、递归秘密字段拒绝；Gateway
  intent 故障失败关闭。

### 垂直链

- [x] 结构化任务到 Stub、artifact、manifest、needs_review、投影和审计 outcome 全链通过。
- [x] 已定义的 Run/report/commit 重放不增加 attempt/artifact/manifest；create_task 没有
  Dispatch 幂等身份，不把重复新建草稿说成幂等。
- [x] orphan 无 manifest 不进入可信结论。

## 3a. T-027 复审修复第一片（2026-09-05，补记）

主线当时未回写本页，此处按 TASKS/INDEX 与远端 CI 记录补记：HEAD `d24e56a`（已快进合并并推送
main）。macOS arm64：fmt / Clippy / test 106 项通过、`cargo doc` 通过；Docker Linux aarch64：
workspace 114 项通过（含 8 项 Linux 路径测试），另显式复跑 `linux_path_security` 8/8——**不重复
计入总数**，档案里的「122」是两次运行相加的口径。远端 CI run 34002573561 与 34004526486 通过。

## 3b. T-032 复审修复第一片（2026-09-06）

日期/时区：2026-09-06 UTC。环境：macOS arm64、Rust 1.97.1（Homebrew）。代码位置：
`.claude/worktrees/m0-m2-code-review-test-8933fd`，分支 `claude/m0-m2-code-review-test-8933fd`，
基线 `d24e56a`；验证时工作树含本片全部未提交改动（见下）。

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0，无 warning |
| `cargo test --workspace --locked --no-fail-fast` | **127 项通过、0 失败**（含 T-029 回合的 gateway 等待后撤权 9 项、control-db schema 守卫 7 项，以及本片新增：gateway 3、task-engine 1、vault-broker 1、core/audit-ledger 用例扩充） |
| `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps --locked` | exit 0 |
| Linux | 本机 Docker 守护进程未运行，未复跑；以推送分支后的远端 CI 为准 |

改动范围：回合脏树已验收的 T-029 `gateway/{admission,wait_probe,waiting_tests}`、`control-db/schema_guard`
与两份契约；Gateway 失败终态封口/占位守卫/结果保留上限（`MAX_RETAINED_RESULTS`，新错误
`CallResultEvicted`）；TaskEngine 投影逐任务隔离（`ProjectionIncomplete`，Run 只交付本任务）；
`VaultPath` 只接受 NFC 且 Broker 碰撞键先 NFC（新依赖 `unicode-normalization 0.1.25`）；审计词法
复数形态与凭据本体后缀。未做：真实 Runner/外发、Linux 本机复跑。

## 3c. T-033 M3 首片（2026-09-07）

日期/时区：2026-09-07 UTC。环境：macOS arm64、Rust 1.97.1、Python 3.12（tiktoken 0.9.0 作独立复核）。代码位置：
`.claude/worktrees/m3-cold-start-supply`，分支 `claude/m3-cold-start-supply`（基线＝WIP `4bdb5e0` + main `639b529`），
提交 `d0dbd97`（supply 内核）、`540d591`（agent-server）及本次文档提交。

| 检查 | 结果 |
|---|---|
| `cargo fmt --all -- --check` | 仅 T-030 在途的 `architecture_cases/{auth_http,canonical_owner}.rs` 有差异；本片文件全部通过 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 仅 T-030 在途的 `local-server`（未用导入、多余 return）报错；`supply`、`agent-server`、`studiolo` 通过 |
| `cargo test --workspace --no-fail-fast` | **836 通过 / 2 失败**，失败均为 T-030 在途 `writer_http_contract::failures`；本片新增：supply 13（roots 3、signpost 3、agent_store 4、budget 2、audit 1）、agent-server 5（options 2、mcp_contract 3 真实 loopback）、architecture +1（supply_storage）|
| `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps` | exit 0 |
| 真实库验收（本机） | `studiolo-agent serve --vault "<原 vault>" --allow 项目/Studiolo --allow _规则`：首扫 92 篇笔记、0 诊断；`initialize` 返回路牌 1583 字符，**o200k 931 token**（cl100k 1105），裁剪等级 0；`tools/list` 4 项；`read_note` INDEX 返回 revision＋正文；`read_note School/README.md` → 拒绝并审计 `outside_allowlist`；`search 冷启动` 45 命中；`list_entries _规则` 18 条；`admin reads` 按类型/实例/路径查到全部读取 |
| Linux | 未复跑（Docker 未运行）；分支已推送，远端 CI 因 T-030 在途 fmt/clippy 预期为红 |

主人接入（2026-09-07 02:32 UTC）：`claude mcp add --transport http studiolo http://127.0.0.1:4400/mcp --header …`（本地作用域，project `/Users/maxsong`）；在该目录下 `claude mcp list` 显示 `studiolo … ✔ Connected`，服务端 `agent_instance` 新增第三个实例 `01M1WVB2P3CQ6RG9FZ5WWWFXWA`（claude），即 Claude Code 自身完成了 initialize 握手。**「一个 URL 填进 Claude Code」首次成立。** 未做：Tunnel/Access 机器 token 部署；推模式与 HTTP 文本兜底；作用域为项目本地，跨目录使用需 `--scope user` 重加。

## 3d. T-034 M3 部署整合与复验（2026-09-07）

接手基线 `ad04c02`，在 `codex/m3-merge-deploy` 整合 Claude 未提交的服务器部署文件。服务器原发布已经运行，
本轮复验并补齐脚本与 CI 门禁；详见 [服务器记录](../m3/server-agent.md#2026-09-07-codex-合并复验)。

| 检查 | 结果 |
|---|---|
| 服务器原生发布包 | SHA256SUMS/二进制/服务单元摘要匹配，475 个源码文件匹配归档；47 个程序、278 项通过 |
| 严格 MCP 验收 | 公网与回环的完整四工具链、白名单拒绝、无/错 token 的 401 均通过 |
| 运维脚本回归 | MCP 检查器 8 项、同步脚本 5 项通过；bash 语法检查通过 |
| 既有 T-030 修复 | 仅超限 writer journal 与读取 I/O 故障补标 stale；writer_http_contract 7/7、ordinary_read_rejections 1/1 |
| fmt / Clippy | workspace 格式及 `--all-targets --locked -- -D warnings` 均通过 |

未重启线上服务或部署 M2；M3 功能代码与原运行包相同，仅更正信号处理注释。Cloudflare Access、边缘兼容规则、
副本自动同步、推模式与 HTTP 文本兜底仍属后续范围。

## 4. 运行记录模板

```text
日期/时区：
环境：macOS arm64 | Linux x86_64（内核/容器版本）
HEAD 与 worktree 状态：
命令：
退出码：
通过/失败/跳过数量：
关键证据：
未验证项：
```
