# Studiolo 仓库 agent 约定（AGENTS.md）

> 适用于在本仓库工作的一切 AI 会话与其子 agent：Claude Code 经根 `CLAUDE.md` 引入本文件，Codex 直接读本文件。
> 本文件只管**派发任务书**与**回执**的格式。并发协作 SOP、预算纪律、模型与 effort 分档一律以全局 `~/.claude/CLAUDE.md` 为准；项目档案在 Obsidian vault `项目/Studiolo-Docs/`，开工先读 `00-总览/INDEX.md` 与 `04-协调/TASKS.md`。
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
- 用**最窄的证明**自检，但测试与编译**在哪跑看你在哪台机器**（对照表见第七节）。**Mac：所有测试与编译检查一律不在本机跑**（2026-09-23 用户硬约束），定向测试、fmt、clippy、build 都用 `scripts/test-linux.sh <命令>`（如 `scripts/test-linux.sh cargo --locked test -p <包> --test <目标>`）在服务器上跑，前端检查以第五节的 ci 套为准。**DGX 2026-11-20 起不再用来开发和测试**（第七节），不在上面跑任何测试与编译。Cursor Cloud VM 见第六节。全量回归留给派发方；全量命令见 `README.md`「开发」段。
- 被卡就在回执里写明阻塞，不猜、不绕、不伪造闭环。
- 临时产物放会话 scratchpad，用完清理自己产生的。

## 五、合并门禁：测试箱的 ci 套（2026-11-16 起，取代 GitHub CI）

- **GitHub CI 和自托管 runner 已弃用**（2026-11-16，spark-testbox 项目 T-078 的 Studiolo 份，用户拍板）：`.github/workflows/ci.yml`、`deploy/ci-runner/` 从仓库删掉，
  GitHub 上的 runner 注销、仓库变量 `CI_RUNNER_LABELS` 删掉。原来那份工作流的三个作业（rust、frontend、alerts-worker）命令一字不改，
  搬进工作树根 `.testbox.toml` 的 `ci` 套。推送和 PR 不再触发任何检查。
- **合进 main 之前必过 ci 套**：在 Mac 上交 `testbox --remote testbox@testbox-ctl submit --project studiolo --recipe full --steps ci --arch x86_64 --memory 20G --cpu 8 --wait`，
  整条放后台等（控制端是 Mac Studio 上的虚拟机 `testbox-ctl`，执行节点只剩 x86_64 的 x86-1，即 108；旧写法 `testbox@max-spark` 也指到控制端，不再用）。一步失败 `--wait` 马上返回并点名那一步，任务照跑完其余步骤（要一次拿全用 `--wait-all`）。
  全量同一时间只许一个会话跑：交之前先看 `testbox status` 有没有别的 studiolo 全量在跑。
- **ci 套是门槛，不是调试器**：改的过程中用定向命令（`testbox --remote testbox@testbox-ctl submit --project studiolo --arch x86_64 --cmd '…'`）验，攒成一批再交 ci 套。
  Mac 本机不跑任何测试与编译检查（`cargo test`／`fmt`／`clippy`／`build`、Python 契约、前端 tsc／lint／vitest／build、真实管线都算，2026-09-23 用户硬约束）。
- 门禁命令由架构守卫逐行钉住（`crates/studiolo/tests/architecture_cases/ci.rs`）：删掉、注释、改参数、吞失败、关 errexit、换目录、
  改成后台服务都判红；有门禁的那一步第一行必须是对应目录的 `source …testbox-ci-env.sh`，最后一条门禁之前只许有别的门禁和几行版本诊断
  （插 `exit`、`if`、`cd`、改 PATH、同名函数也判红）。新加的回归放在那一步最后一条钉住的行之后，或者先在守卫里钉上。
  改 `.testbox.toml` 只用守卫认得的写法（见文件开头说明）。
- 沙盒和原来 runner 的环境差别（umask、私有临时目录、本任务自己的构建目录、Node 版本）由 `scripts/testbox-ci-env.sh` 补齐，
  每一步开头 source 它；它只在测试箱里用，会换掉 HOME、TMPDIR 和构建目录。
- **Mac 会话的 Linux 回路照旧在 `githubci` 上原生跑，不用 Docker**（2026-09-19，T-081）：`scripts/test-linux.sh` 把当前工作树 rsync 到服务器后原生执行
  和 ci 套相同的 Rust 门禁，每个工作树一份远端同步树与构建目录、flock 排队。它只是快速回路，合并门槛仍是 ci 套。
  别往仓库里加容器作业或 `docker run`（发布打包除外）。
- 决定与取舍：vault `03-记忆/ADR-036-合并门禁改到测试箱ci套.md`（取代 ADR-020）；跨项目的总决定是 spark-testbox 的 ADR-021。

## 六、Cursor Cloud 云端 VM（2026-09-27 起）

- 云端 VM 是一次性的 Ubuntu 24.04 x86_64（不是第四节写的 Mac），与测试箱同为原生 Linux，但连不到 `githubci` 和测试箱，`scripts/test-linux.sh` 在这里不可用。
  改用 **`scripts/test-cursor-cloud.sh <命令>`** 原生跑（对应 `test-linux.sh` 带参数的用法）；默认回路按 `test-linux.sh` 里的清单逐条跑，其余步骤照 `.testbox.toml` 的 ci 套。
- 必须经这个脚本跑。所有云端 VM 的可锁定内存上限都只有 8 MiB，普通用户抬不动，秘密驻留面加固（mlockall）会失败关闭（如 `studiolo-local-server` 二进制里的 `startup::tests` 集体红）；
  没有环境构建、从默认镜像启动的 VM 根文件系统还是 overlayfs，目录与文件的 `st_dev` 不同，vault-broker 与 Python 副本／快照／发行工具的同设备检查对 `/tmp`、`$HOME` 下的夹具会失败关闭（从环境构建启动的是普通 ext4）。
  脚本抬高该上限，并在私有 mount 命名空间里给 `/tmp` 与 `HOME` 换上临时 ext4，两种 VM 都适用。裸跑看到的这两类失败是环境问题，**不要去改这些检查**。
- 已知环境局限：`studiolo-vault-broker` 的 `metadata_to_body_changes_do_not_publish_partial_or_missing_results` 与 `missing_and_empty_anchor_evidence_must_survive_until_final_revalidation`
  在 VM 上几乎必然失败——内核时间戳粒度 4 ms，同一节拍内的等长改写或删目录不改变 mtime／ctime。这两条以 ci 套为准；
  全量测试用 `scripts/test-cursor-cloud.sh cargo --locked test --workspace -- --skip metadata_to_body_changes --skip missing_and_empty_anchor`。
- 前端检查不需要这个脚本：`npm --prefix web run build`／`test`／`lint`／`format` 直接跑（`node` 为 22.x，ci 套钉 22.17.0）。
  只看界面可直接 `STUDIOLO_FIXTURES=1 npm --prefix web run dev`；要连真后端，README 的本机只读示例也得经这个脚本，且示例库与构建产物先复制进它给的 ext4（否则扫描不完整、不监听）：
  `scripts/test-cursor-cloud.sh sh -c 'cp -R examples/m1-vault web/dist /tmp/ && exec cargo run --locked -p studiolo-local-server -- --vault /tmp/m1-vault --assets /tmp/dist --label 示例库 --port 4317'`。
- 线上站点 `https://studiolo.maxsong.me` 在 Cloudflare Access 单邮箱验证码门禁之后，应用层还校验 Access 令牌里的精确邮箱：VM 能连通，但未登录请求一律 302 到登录页，
  agent 无法登录（service token 也过不了应用层）。界面验证用上面的本机示例或假数据。
- 环境安装脚本（保存在 Cursor 面板，不在仓库）已备好 1.97.1 ＋ rustfmt／clippy、GNU `time`（内存测量用例调 `/usr/bin/time`）、`web/node_modules` 与 cargo 依赖缓存，
  并在 `$CARGO_HOME/config.toml` 设 `[profile.dev] debug = "line-tables-only"`，与 ci 套、`test-linux.sh` 同一设置。
- vault `项目/Studiolo-Docs/` 与全局 `~/.claude/CLAUDE.md` 不在 VM 上。GitHub 上已没有 CI（第五节）：推送不触发检查，合并门禁 ci 套只能由本机会话交测试箱，VM 里连不到。
- 在推进工作的交接快照放在 `docs/handoff/`（vault 为准，可能滞后）。验证在 VM 里一律用上面的 `test-cursor-cloud.sh`，合并前的 ci 套写明「交本机会话」；要登录 107／108／169、读写 vault 或用 studiolo MCP 的步骤云端做不了，停下写明「交本机会话」（其中登录 107／108／169 的只能交 Mac 会话），不要索要或注入内网／SSH 凭据。

## 七、在哪台机器上干活（DGX 2026-11-20 起退出开发与测试）

| | Mac（MacBook、Mac Studio） | Cursor Cloud VM（第六节） |
|---|---|---|
| 系统 | macOS arm64 | Ubuntu 24.04 x86_64，一次性 |
| 本机跑测试与编译 | 不跑（2026-09-23） | 可以；除前端检查外都经 `scripts/test-cursor-cloud.sh` |
| 连得到 `githubci` 与家里 192.168.100.x | 经 Tailscale 子网路由 | 连不到 |
| Linux 回路入口 | `scripts/test-linux.sh [命令]`（同步到 `githubci` 跑）；测试箱 `testbox --remote testbox@testbox-ctl …`（第五节） | `scripts/test-cursor-cloud.sh <命令>` |

- **DGX（`max-spark`）不再用来开发和测试**：2026-10-04～11-20 它曾是第二台开发机和测试机（ADR-028，已被取代）。用户 2026-11-16 定开发搬到 Mac Studio、DGX 只当测试机和跑大模型（spark-testbox ADR-025），
  2026-11-20 又把它从测试箱彻底摘掉、整台留给大模型（spark-testbox ADR-026）。不在 DGX 上开发、跑测试或编译，也不往那里派活；原来 `README.md`「开发」段的「DGX 本机跑」随之删掉（最后一版见提交 `f2e8639`）。
- 测试箱只剩 x86_64 节点 x86-1（108）：不带 `--arch` 的任务落 108，`--arch aarch64` 提交即被拒。本仓库在 aarch64 上没有地方跑测试，
  需要时如实写「aarch64 没地方跑」，不改派到 x86_64 冒充；108 掉线时没有备用，任务排队等它回来。
- 部署与发布只在 Mac 做，vault 副本也只由 Mac 推（2026-10-04 用户拍板）。
