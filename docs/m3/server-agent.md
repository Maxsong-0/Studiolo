# M3 · 服务器常驻 `studiolo-agent`

T-034：用户于 2026-09-07 指定部署到 `maxsong@192.168.100.169`，把 M3 首片（[供给面与 MCP 外壳](../contracts/agent-supply.md)）
从 Mac 上的临时进程变成服务器常驻服务，并经专用 Tunnel 给出一个公网 URL。
真实 vault 仍在 Mac：服务器只持有**白名单子树的只读副本**（`项目/Studiolo`、`_规则`），
不复制 vault 其他内容，不提供写入，不改变 M2 未完成的事实。副本会陈旧，见「副本同步」。

**2026-09-11 更新（T-040）**：已上线 `m3-t040-20260910-read01`，同一系统服务新增 HTTP 文本入口和独立持久索引，系统及公网 MCP/HTTP 正反验收通过；客户端投影 CLI 随源码包发布。当前版本、验收和恢复 T-036 系统单元的备份见 [读取供给发布](read-supply-release.md)。

**2026-09-10 更新（T-036，历史）**：已上线 `m3-t036-20260910-replica01`，系统级 `studiolo-agent.service` 以 maxsong 运行并读取托管副本；旧用户单元已禁用。Mac 每 60 秒自动同步已启用，原生302项与公网正反验收通过。资料同步证据见 [资料同步验收](replica-freshness.md)，同步运维见 [系统级操作说明](replica-operations.md)。以下 T-034 段落保留为历史。

## 边界与已核实环境

- 服务器与 [T-008](../m1/server-preview.md) 相同：Ubuntu 26.04 x86_64、用户级 systemd 259、Linger=yes；无 Rust/Node/Docker。
- 既有 `studiolo-preview.service`（4318）、`studiolo-access.service`（4319）、`studiolo-tunnel.service` 不停止、不重启、不改单元。
- 新服务只监听 `127.0.0.1:4400`；公网入口是专用隧道 `studiolo-preview`（ID `4c425eda-…`）新增一条 ingress：
  `studiolo-agent.maxsong.me → http://127.0.0.1:4400`，HTTP Host 覆写 `127.0.0.1:4400`。既有 `studiolo.maxsong.me` 规则与 404 兜底不动。
  主机名必须是 `maxsong.me` 的一级子域：Cloudflare 通用证书只覆盖 `*.maxsong.me`，最初试的二级子域 `agent.studiolo.maxsong.me`
  在边缘 TLS 握手即失败（不买 Advanced Certificate 就无法用二级子域），已删除该 DNS 记录并改名。
- 门禁在应用内：每个请求必须带 `Authorization: Bearer <类型 token>`，缺失/无效 401 并写 `agent.action_denied` 审计。
  **未加 Cloudflare Access**：服务 token 的秘密只在创建时显示一次，agent 不能经手；主人可在面板自行签发后再挂 Access 应用，见「后续」。
- 私有库 `~/.local/share/studiolo-agent`（0700，`agent.sqlite3` 与 `audit/audit.sqlite3`）是服务唯一可写路径；
  发布目录与副本目录经 `ProtectHome=read-only` 保持只读。`RestrictSUIDSGID=no` 沿用 T-008 的 openat2 结论。
- `serve` 现同时响应 SIGINT 与 SIGTERM 优雅关停（systemd 默认 `KillSignal=SIGTERM`），T-034 补齐。

## 目录约定（服务器）

| 路径 | 用途 | 权限 |
|---|---|---|
| `~/.local/share/studiolo-agent/` | 私有库与审计账本（`--data`） | 0700，文件 0600 |
| `~/.local/share/studiolo-agent-vault/` | 白名单子树只读副本（`--vault`） | 整棵 0700/0600 |
| `~/.local/share/studiolo-agent-releases/<发布名>/` | 运行包：`bin/`、`tests/`、`source/`、`SHA256SUMS` | 0700 |
| `~/.config/studiolo-agent/claude.token` | `claude` 类型 token 唯一明文副本（也拷回 Mac 一份供 Claude Code 使用） | 0600 |
| `~/.config/systemd/user/studiolo-agent.service` | 由 `deploy/agent/studiolo-agent.service.in` 替换 `@RELEASE@` 生成 | — |

## 副本同步（Mac）

```bash
bash scripts/sync-agent-vault.sh            # 加 --dry-run 只列差异
```

T-036 将此入口升级为完整批次发布：源端双次稳定捕获，SSH 接收端逐项校验清单和 SHA-256，
然后原子发布到新的 `~/.local/share/studiolo-agent-replica/`。仅包含原白名单两个目录，隐藏子树完全跳过，
链接、特殊文件、非 NFC 路径和不完整传输均拒绝；目录 0700、文件 0600。`--dry-run` 现在是纯本地校验，不连接服务器。

已部署的 T-034 单元仍读旧 `studiolo-agent-vault/`；新入口不修改该目录。
须在首批同步后安装新的 `--replica-root` 单元才能读取新副本。自动任务、独立端口验证和回滚步骤见
[自动同步运维](replica-operations.md)，完整协议见 [资料同步与新鲜度](replica-freshness.md)。
它仍不是 ADR-005 的 canonical 同步桥，也不是 ADR-015 向客户端下发规则的推模式。

## 构建匹配源码的运行包（Mac）

```bash
mkdir -p target/agent-packages
bash scripts/package-agent.sh "$PWD/target/agent-packages/<唯一发布名>"
```

与 `package-preview.sh` 同一套约束：显式源码白名单（含架构守卫要读的 `package-preview.sh`/`test-linux.sh`/`ci.yml`）、
固定 `rust:1.97.1-bookworm` amd64 镜像 digest、每个发布名独立目标卷、`SHA256SUMS` 覆盖整包；
`provenance.txt` 记录快照是否与 `base-commit` 一致。随包携带的测试程序只包含 `studiolo-agent` 实际链接的 crate
（supply、agent-server、vault-broker、core、audit-ledger、vault-index、workbench）加 `studiolo` 架构守卫；
解析步骤要求 `architecture`、`linux_path_security`、`mcp_contract`、`roots_contract`、`signpost_contract`、`agent_store_contract` 六个目标齐全。
local-server 等未随包发布的 crate 测试由 CI 与各自运行包负责。

## 历史用户服务安装（T-034，直接副本模式）

以下保留为旧 `--vault` 服务的安装记录。T-036 托管副本须使用系统级单元，见 [自动同步运维](replica-operations.md)；不得按以下用户级步骤切换 `--replica-root`。

```bash
# 1. 预建目录（一次性）
umask 077; mkdir -p ~/.local/share/studiolo-agent/audit ~/.local/share/studiolo-agent-releases ~/.config/studiolo-agent
# 2. 放入运行包并核对整包
cd ~/.local/share/studiolo-agent-releases/<发布名> && sha256sum --check SHA256SUMS
mkdir source && tar -xzf source.tar.gz -C source
# 3. 原生测试（任一失败即停止，不安装）
cd source && bash ../run-agent-tests.sh
# 4. 私有库初始化与类型 token（token 只打印一次，落到 0600 文件，不进聊天）
../bin/studiolo-agent admin init --data ~/.local/share/studiolo-agent
../bin/studiolo-agent admin type-issue --data ~/.local/share/studiolo-agent --type claude | tail -1 > ~/.config/studiolo-agent/claude.token
# 5. 安装并启动用户服务
install -m 644 ../studiolo-agent.service ~/.config/systemd/user/studiolo-agent.service
systemctl --user daemon-reload && systemctl --user enable --now studiolo-agent.service
```

运维：`systemctl --user status studiolo-agent.service --no-pager`、`journalctl --user -u studiolo-agent.service -n 30 --no-pager`；
读取审计查询在服务器上执行 `bin/studiolo-agent admin reads --data ~/.local/share/studiolo-agent …`（数据库只在服务器）。
更新发布：新发布名重跑打包与第 2–3 步，替换单元里的发布名后 `systemctl --user restart studiolo-agent.service`；
`agent.sqlite3` 与账本跨发布沿用。停止：`systemctl --user disable --now studiolo-agent.service`。
吊销/轮换 `claude` 类型：`admin type-revoke --type claude` 后重新 `type-issue` 并更新两端 token 文件。

## 接入（Mac 上的 Claude Code）

```bash
claude mcp add --scope user --transport http studiolo https://studiolo-agent.maxsong.me/mcp \
  --header "Authorization: Bearer $(cat ~/.local/share/studiolo-agent/claude-server.token)"
```

服务器签发的 token 与本机 `~/.local/share/studiolo-agent/claude.token`（本机实例）不是同一个；两处私有库各自独立。

## 实际部署记录

2026-09-07（服务器日志 UTC 03:22–03:31）：部署完成，并经公网真实客户端验收。

- 发布目录：`/home/maxsong/.local/share/studiolo-agent-releases/m3-t034-20260907-agent01/`；本机构建包
  `target/agent-packages/m3-t034-20260907-agent01/`（Docker amd64 仿真，镜像 `rust@sha256:0e2bcaef…`，rustc 1.97.1）。
  源码快照含 Claude 当时尚未提交的部署改动，`base-commit` 为 `ad04c02`；后续合并与复验见下节。
- 服务器原生：整包 `SHA256SUMS` 全部匹配；**47 个 Rust 测试程序 278 项全部通过**（含 `architecture`、`linux_path_security`、`mcp_contract`）。
- `studiolo-agent.service`：active/enabled、NRestarts=0，只监听 `127.0.0.1:4400`；ProtectSystem=strict、ProtectHome=read-only、
  ReadWritePaths=私有库、NoNewPrivileges、MemoryMax 512 MiB、TasksMax 64。首扫 93 篇笔记、0 条诊断。既有 4318/4319/tunnel 三服务全程 active，未重启。
- 回环验收（`scripts/check-agent.py`，脚本经 stdin 送到服务器，token 不离开服务器）：initialize 200 带会话 id、路牌 1646 字符（首行 `# Studiolo 路牌`）、
  四个 tools、`handshake` 与 `read_note` INDEX.md（10229 字符）通过、白名单外 `-32602 路径不在供给白名单内`、
  无 token / 错 token 均 401 且 `WWW-Authenticate: Bearer realm="studiolo-agent"`；`admin reads` 查到该次读取。
- Cloudflare：DNS `studiolo-agent.maxsong.me` CNAME → 隧道（proxied）；隧道配置 version 3（连接器日志 `Updated to new configuration … version=3`）。
  最初的 `agent.studiolo.maxsong.me` 因通用证书不覆盖二级子域而 TLS 握手失败，该记录已删除。
- 公网验收（Mac）：同一脚本走 `https://studiolo-agent.maxsong.me/mcp` 全部通过（响应带 `cf-ray`）；无 token 401；本机实例的 token（另一私有库）401。
  **真实客户端**：`claude -p --mcp-config`（临时配置文件、不落用户配置，Opus 5）MCP 状态 `connected`，调用 `handshake` 取回路牌代次 9；
  服务器登记实例 `01M1WYPR6G96P6S7X651DNN8N0`，账本有对应 `agent_instance.registered` 与 `handshake.served`。
- 边缘保护观察：zone 的 Super Bot Fight Mode「Definitely automated = managed_challenge」会把 curl 默认 UA 与 Node 默认 UA（`node`）的请求
  拦成 `403 cf-mitigated: challenge`；Claude Code 的客户端未被拦。按 zone 既有惯例（API 路径＋Authorization 头跳过 SBFM）拟加一条 WAF 自定义规则
  `(http.host eq "studiolo-agent.maxsong.me" and has_key(http.request.headers, "authorization"))` → Skip（SBFM、BIC、security level）；
  agent 会话的自动模式分类器拒绝了这项安全配置改动，**留待主人在面板决定**。未加此规则前，其他 AI 客户端（Codex/Grok 等）能否直连取决于其 UA 是否被判为「definitely automated」。
- 未做：Cloudflare Access（服务 token 秘密不经 agent）、推模式副本、副本自动同步。

可复核校验值（SHA-256）：

| 对象 | 值 |
|---|---|
| 运行包 `SHA256SUMS` | `3d8d80b47571fc3b499ff68acf13e24c0c5ea438a0969f7d813b20ea7e20aed5` |
| 服务二进制 `bin/studiolo-agent` | `73012d856f48a7d90b94c85ae88d3507528887419e0e34b955ff39e166dca94c` |
| 已安装单元 `studiolo-agent.service` | `e3390a5536003772a22482b2153663228755f871fc388c7e7bd2cf71f208660f` |

## 2026-09-07 Codex 合并复验

接手时远端 `main` 与 Claude 分支都为 `ad04c02`：T-033 已合并；T-034 的部署文件仍在 Claude 工作区未提交，
服务器已经运行上述 `m3-t034-20260907-agent01`。本轮在 `codex/m3-merge-deploy` 整合，保留 Claude 原工作区。

- 重新校验服务器整包和服务单元，摘要与上表一致；发布目录的 475 个源码文件逐个匹配 `source.tar.gz`。
  服务器原生再次通过 47 个程序、278 项测试。服务 active/enabled、NRestarts=0、仅监听回环 4400。
- 收紧 `check-agent.py`：每一步断言 HTTP/JSON-RPC/工具结果；四个只读工具全部实际调用；无/错凭据只认 401＋Bearer，
  越界读取只认明确的白名单拒绝；禁止重定向且不输出 token、会话 id 或笔记正文。新增 8 项脚本回归。
- 新脚本在回环与公网各通过 initialize → initialized → tools/list → handshake → read_note → list_entries → search → 越界拒绝；
  无 token、错误 token 在两端分别返回 401。路牌 1648 字符，INDEX 读取结果 10229 字符。
- 修正 `sync-agent-vault.sh --dry-run` 的 mkdir/chmod 副作用，隐藏目录整棵跳过，多余参数拒绝；5 项隔离回归通过。
  两组脚本回归纳入 CI；后续运行包携带严格验收脚本。
- 修复 main 既有 T-030 的两处 CI 规范错误与两项读取故障漏标 stale；`writer_http_contract` 7/7，普通读取拒绝回归通过。
  此修复属于本地工作台，不在已部署的 `studiolo-agent` 依赖链内，不代表完整 M2 完成或部署。
- 整合分支 CI 34148098748 全绿（Linux workspace 850 项）。main 复跑 34148811424 暴露既有 PTY hangup 测试遗漏：
  终端断开时可以先报 Io，恢复成功后才收到 HUP。测试仅额外接受 `Err(Io);RESTORED=true`，
  无验证码泄露、退出状态与挂断信号检查保留，生产代码不变。
- M3 运行代码与原发布源码相同（仅 SIGTERM 注释更正）；本轮改进集中在运维脚本、CI 和非发布依赖。
  因此继续使用已校验的原不可变发布包，未重建二进制、重启服务、改动边缘保护或同步真实副本。
  构建快照不是最终合并提交的全仓库快照；以原发布包校验和和本节差异说明追溯。

本机复验日志位于 `target/m3-integration-checks/`（不提交）；远端 CI 结果另见本次合并提交的 Actions。
