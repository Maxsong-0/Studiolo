# Studiolo 仓库 agent 约定（AGENTS.md）

> 适用于在本仓库工作的一切 AI 会话与其子 agent：Claude Code 经根 `CLAUDE.md` 引入本文件，Codex 直接读本文件。
> 本文件只管**派发任务书**与**回执**的格式。并发协作 SOP、预算纪律、模型与 effort 分档一律以全局 `~/.claude/CLAUDE.md` 为准；项目档案在 Obsidian vault `项目/Studiolo/`，开工先读 `00-总览/INDEX.md` 与 `04-协调/TASKS.md`。
> 来源：2026-09-17 借鉴 Droppy Code Hydra 的任务书与回执纪律（vault `03-记忆/Droppy-Hydra多agent协同机制要点.md`）；机器约束版见 ADR-017。黑板上线前，本文件是过渡性的文本约定。

## 一、派发任务书（给每个子 agent 的 prompt 必含，缺一项不派）

1. **目标**：一句话，二元可判。
2. **步骤**：编号的机械步骤，每步写清 `文件路径:行锚`、符号名、精确改动（代码形状／新名字／新值）。**不许提问，不许给两个备选，不许「先通读再判断」**。没定的事派发方先定，或留给自己。
3. **独占文件集**：本任务允许写的路径列表。并行派发的各任务文件集**两两不相交**；任务书没点名的文件不许碰。
4. **验收标准**：逐条二元可判，写明验证命令。本仓库标准回路见 `README.md`「开发」段。
5. **禁区**：不许碰什么、不许做什么。至少包含：不 commit／push／切分支／stash，不改共享 `target/`，不启停共享服务，不回滚或整理别人的改动。
6. **必读文件**：路径列表，不是「去读这个项目」。
7. **回执要求**：落盘 `REPORT.md`，字段见第二节。子 agent 不一定 recall memory，协作约束必须写进 prompt。

## 二、回执（`REPORT.md` 固定字段，缺一项视为未完成）

| 字段 | 内容 |
|---|---|
| 状态 | `completed` ／ `failed` ／ `stopped`（被打断） |
| 改动文件 | 每个路径一行，附增删行数；只列自己改的 |
| 冲突 | 与他人改动撞上的路径；非空则不得宣称完成 |
| 未落地 | 做了但没进工作树的改动（patch 路径） |
| 验收自评 | 逐条对应任务书验收标准：`pass` ／ `fail` ／ `unverified`，附实际跑过的命令与关键输出行 |
| 剩余事项 | 二元可判，供接棒 |
| 耗时与工具数 | 粗略即可 |
| 学到什么 | 可空；有则由派发方分流进 vault `03-记忆/` 或 agent memory |

自然语言流水账不算回执。

## 三、并行派发规则（派发方）

- 一任务一 agent，一文件一持有者；先按第一节写全任务书再派，不许「剩余项／相关优化／某模块完善」式模糊派发。
- **不派 agent 去验证或重做另一个 agent 的活**；整合、最终验收与回答由派发方自己做一次（全局纪律「验收一次做完」）。
- 派发轮次上限 3（研究一轮、改码一轮、补漏一轮），并发上限 8。
- 派发前先读 vault `04-协调/TASKS.md` 查重并登记写域；子 agent 的写域是父写域的子集。

## 四、执行方纪律（子 agent）

- 只做任务书里的这一件，不扩范围；任务书没点名的文件非改不可时，最小改动并在回执里说明。
- 工作树里有别人的未提交改动是正常的：不 stash、不 checkout、不 reset、不 restore、不 clean；不用 `git status`／`git diff` 判断「谁改了什么」。
- 用**最窄的证明**自检，但测试与编译**在哪跑看你在哪台机器**（对照表见第七节）。**Mac：所有测试与编译检查一律不在本机跑**（2026-09-23 用户硬约束），定向测试、fmt、clippy、build 都用 `scripts/test-linux.sh <命令>`（如 `scripts/test-linux.sh cargo --locked test -p <包> --test <目标>`）在服务器上跑，前端检查以 CI 为准。**DGX：子 agent 不跑任何测试与编译**（2026-10-04 用户拍板），把要跑的命令写进回执「验收自评」并标 `unverified`，由主会话最后统一跑。Cursor Cloud VM 见第六节。全量回归留给派发方；全量命令见 `README.md`「开发」段。
- 被卡就在回执里写明阻塞，不猜、不绕、不伪造闭环。
- 临时产物放会话 scratchpad，用完清理自己产生的。

## 五、CI（自托管 runner，2026-09-18 起）

- **CI 跑在专用服务器 `githubci`（192.168.100.107，Ubuntu x86_64）上的自托管 runner**，不再消耗 GitHub 托管分钟数。
  开关是仓库变量 `CI_RUNNER_LABELS`：存在即走自托管，删掉即回退 GitHub 托管。**不要自行改动或删除这个变量**——私有仓库的免费额度两天就能烧光。
- **CI 是门槛，不是调试器**。推送前先把 Linux 那一半验完：Mac 上用 `scripts/test-linux.sh` 在服务器上跑，**Mac 本机不跑任何测试与编译检查**（`cargo test`／`fmt`／`clippy`／`build`、Python 契约、前端 tsc／lint／vitest／build、真实管线都算，2026-09-23 用户硬约束）；DGX 上由主会话在本机原生跑（第七节）；Cursor Cloud VM 用 `scripts/test-cursor-cloud.sh`（第六节）。
  一个 runner 一次只跑一个作业，所有会话共用一条队列：反复小步推送等于让别人排队。攒成一次推送；不为 WIP 开 PR。
- `push` 只对 `main` 触发，PR 走 `pull_request`，同一 PR 上被顶替的旧运行会自动取消。不要把触发改回 `branches: ["**"]`。
- runner 离线或作业一直排队：先 `gh api repos/Maxsong-0/studiolo/actions/runners` 看状态并报告用户。它是共享服务，
  只有确认空闲时才可用窄 sudo 助手重启：`ssh maxsong@192.168.100.107 'sudo /usr/local/sbin/ci-runner-service restart studiolo'`（只能从连得到家里 192.168.100.x 网段、且有登录密钥的机器做——目前只有 Mac；DGX 两样都没有）。
- 这台机器只挂私有仓库、不放任何生产凭据。runner 注册令牌只由用户本人经 stdin 送入，agent 不经手、不打印。
- **Mac 会话的 Linux 回路也在这台服务器上原生跑，不用 Docker**（2026-09-19，T-081；DGX 见第七节）：`scripts/test-linux.sh` 把当前工作树 rsync 到服务器后原生执行同一组检查，
  每个工作树一份远端同步树与构建目录、flock 排队；它**不占 runner 队列**，推 PR 前先用它把 Linux 那一半验完。
  别再往仓库里加容器作业或 `docker run`（发布打包除外）。
- 给别的仓库接入同一台机器：`deploy/ci-runner/README.md`。决定与取舍：vault `03-记忆/ADR-020-CI迁到自托管runner.md`。

## 六、Cursor Cloud 云端 VM（2026-09-27 起）

- 云端 VM 是一次性的 Ubuntu 24.04 x86_64（不是第四节写的 Mac，也不是第七节的 DGX），与 CI 同为原生 Linux，但连不到 `githubci`，`scripts/test-linux.sh` 在这里不可用。
  改用 **`scripts/test-cursor-cloud.sh <命令>`** 原生跑（对应 `test-linux.sh` 带参数的用法）；默认回路按 `test-linux.sh` 里的清单逐条跑，其余 CI 步骤照 `ci.yml`。
- 必须经这个脚本跑。所有云端 VM 的可锁定内存上限都只有 8 MiB，普通用户抬不动，秘密驻留面加固（mlockall）会失败关闭（如 `studiolo-local-server` 二进制里的 `startup::tests` 集体红）；
  没有环境构建、从默认镜像启动的 VM 根文件系统还是 overlayfs，目录与文件的 `st_dev` 不同，vault-broker 与 Python 副本／快照／发行工具的同设备检查对 `/tmp`、`$HOME` 下的夹具会失败关闭（从环境构建启动的是普通 ext4）。
  脚本抬高该上限，并在私有 mount 命名空间里给 `/tmp` 与 `HOME` 换上临时 ext4，两种 VM 都适用。裸跑看到的这两类失败是环境问题，**不要去改这些检查**。
- 已知环境局限：`studiolo-vault-broker` 的 `metadata_to_body_changes_do_not_publish_partial_or_missing_results` 与 `missing_and_empty_anchor_evidence_must_survive_until_final_revalidation`
  在 VM 上几乎必然失败——内核时间戳粒度 4 ms，同一节拍内的等长改写或删目录不改变 mtime／ctime。这两条以 CI 为准；
  全量测试用 `scripts/test-cursor-cloud.sh cargo --locked test --workspace -- --skip metadata_to_body_changes --skip missing_and_empty_anchor`。
- 前端检查不需要这个脚本：`npm --prefix web run build`／`test`／`lint`／`format` 直接跑（`node` 为 22.x，CI 钉 22.17.0）。
  只看界面可直接 `STUDIOLO_FIXTURES=1 npm --prefix web run dev`；要连真后端，README 的本机只读示例也得经这个脚本，且示例库与构建产物先复制进它给的 ext4（否则扫描不完整、不监听）：
  `scripts/test-cursor-cloud.sh sh -c 'cp -R examples/m1-vault web/dist /tmp/ && exec cargo run --locked -p studiolo-local-server -- --vault /tmp/m1-vault --assets /tmp/dist --label 示例库 --port 4317'`。
- 线上站点 `https://studiolo.maxsong.me` 在 Cloudflare Access 单邮箱验证码门禁之后，应用层还校验 Access 令牌里的精确邮箱：VM 能连通，但未登录请求一律 302 到登录页，
  agent 无法登录（service token 也过不了应用层）。界面验证用上面的本机示例或假数据。
- 环境安装脚本（保存在 Cursor 面板，不在仓库）已备好 1.97.1 ＋ rustfmt／clippy、GNU `time`（内存测量用例调 `/usr/bin/time`）、`web/node_modules` 与 cargo 依赖缓存，
  并在 `$CARGO_HOME/config.toml` 设 `[profile.dev] debug = "line-tables-only"`，与 CI、`test-linux.sh` 同一设置。
- vault `项目/Studiolo/` 与全局 `~/.claude/CLAUDE.md` 不在 VM 上。推送 PR 同样占第五节那条共享 runner 队列（draft PR 也触发）：先在 VM 里验完，攒成一次推送。
- 在推进工作的交接快照放在 `docs/handoff/`（vault 为准，可能滞后）。验证一律用 GitHub CI 加上面的 `test-cursor-cloud.sh`；要登录 107／108／169、读写 vault 或用 studiolo MCP 的步骤云端做不了，停下写明「交本机会话」（其中登录 107／108／169 的只能交 Mac 会话：DGX 同样连不到家里 192.168.100.x 网段），不要索要或注入内网／SSH 凭据。

## 七、DGX（`max-spark`，2026-10-04 起）

| | Mac | DGX（`max-spark`） | Cursor Cloud VM（第六节） |
|---|---|---|---|
| 系统 | macOS arm64 | Ubuntu 24.04 aarch64 | Ubuntu 24.04 x86_64，一次性 |
| 本机跑测试与编译 | 不跑（2026-09-23） | 主会话可以跑，子 agent 不跑（2026-10-04） | 可以；除前端检查外都经 `scripts/test-cursor-cloud.sh` |
| 连得到 `githubci` 与家里 192.168.100.x | 经 Tailscale 子网路由 | 连不到（不收子网路由，也没有登录密钥） | 连不到 |
| Linux 回路入口 | `scripts/test-linux.sh [命令]`（同步到 `githubci` 跑） | 手动，按 `README.md`「开发」段「DGX 本机跑」 | `scripts/test-cursor-cloud.sh <命令>` |

- DGX 是用户自己的 DGX Spark，仓库在 `~/All-Project/studiolo`（2026-10-03 从 Mac 原样拷来，之后两边各自干活、经 GitHub 推拉）。本地 `main` 可能落后，开工先 `git fetch`，从 `origin/main` 开分支。
- 主会话按 `README.md`「开发」段「DGX 本机跑」在本机原生跑：umask 022、每个工作树一份 `CARGO_TARGET_DIR`（不用共享 `target/`）；
  **全量回路整机一把锁**（`flock -n`，别的会话占着就退出，不等、不抢；锁文件与命令都在那一段）。`scripts/test-linux.sh` 的本机模式另有任务，落地前一律照那段手动跑。
- DGX 不用第六节的 `scripts/test-cursor-cloud.sh`：`ulimit -l` 为 unlimited、`/tmp` 与家目录在同一块 ext4 上（2026-10-04 实测），那两类环境失败在这里不会出现；
  它还要免密 sudo，而 DGX 的 sudo 只有用户本人能输。缺工具就停下报告用户，不要自己下载执行安装脚本；工具链现状、aarch64 与 CI 的差别见 README 那一段。
- 要进家里 192.168.100.x 网段的事（runner 重启、登录 107／108／169、打包发布、vault 副本推送到 169）DGX 做不了，交 Mac 会话。
  部署与发布只在 Mac 做（DGX 不放部署凭据），vault 副本也只由 Mac 推（均为 2026-10-04 用户拍板）。
