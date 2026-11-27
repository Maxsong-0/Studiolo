# Studiolo 仓库规则

@AGENTS.md

权威优先级与并发协作 SOP 见全局 `~/.claude/CLAUDE.md`；项目档案在 Obsidian vault `项目/Studiolo-Docs/`（先读 `00-总览/INDEX.md` 与 `04-协调/TASKS.md`）。

## Bottega 的编译、测试只经 testbox 的 Mac 节点（用户 2026-11-26 定，vault ADR-048；取代 ADR-043 的「本机直接编译、测试」）

- **范围只有三样**：Bottega（仓库 `bottega/`，原生 Mac App）、以后的 Bottega 桥（Studiolo 在 Mac 上的常驻进程），和 P3 的 iPhone App（同一个 Swift 核心包）。它们要验的东西只在 Mac 上有（Secure Enclave、钥匙串、Touch ID、`LOCAL_PEERTOKEN`、代码签名的指定要求），108 编不了也跑不了，所以交 testbox 的 **Mac 节点 `macos-1`**（就是 Mac Studio 本机）。**网页、169 那一侧的代码照旧交 108**（`AGENTS.md` 第四、五节），合并门禁照旧是测试箱的 ci 套；Bottega 不在 ci 套里。
- **在 Mac Studio 上、站在工作树根目录交，整条放后台等**；任务以 maxsong 身份、不加沙盒，在你交的那个目录里原地跑（钥匙串、Touch ID、Xcode 都能用），产物留在原处：
  - 编 Debug 版（配方默认，targeted 档 30 分钟）：`testbox --remote testbox@testbox-ctl submit --arch macos --project bottega --memory 8G --cpu 8 --wait`
  - 编并跑全部测试（`--recipe full`，24 小时档）：`testbox --remote testbox@testbox-ctl submit --arch macos --project bottega --memory 8G --cpu 8 --recipe full --wait`
  - 别的命令：`… --cmd 'scripts/bottega-local swift test --package-path BottegaKit' --wait`
- **一律经 `scripts/bottega-local`**（`build`、`test`、`xcodebuild <参数…>`、`swift <参数…>` 只在 testbox 任务里跑，认 `TESTBOX_TASK_ID`；在任务外敲就拒绝、退 2，并印出该交的命令），不裸跑 `xcodebuild`、`swift`、`xcodegen`。`generate` 只生成工程、不编译，哪里都能跑。
- **起 App 给人看**：testbox 收不到 `open` 起的 App，所以编译在 testbox 里、打开在任务外：先交上面那条编好，再在本机 `scripts/bottega-local run`（不编译，只打开 testbox 编好的那份；编好以后源码又改过就拒绝，免得看旧版）。装进 `/Applications` 用 `bottega/scripts/quick_run.sh`，同样只装 testbox 编好的、只在用户说「装上」时用。
- **只编 Debug**：发布版的签名照旧由用户本人做（ADR-043「不变的」照旧有效），脚本拒绝 Release 和 archive；测试不碰真的 LMAS 运行目录和真的钥匙。
- **只限 Mac Studio**：testbox 只有这一个 Mac 节点，MacBook 交 `--arch macos` 会被拒（退 125），如实写「这台编不了」，不改交 108。节点离线（Mac 睡眠、没登录）就排队等，没有本机直跑的后门。
- 同一目录、同一项目同时只跑一个，后来的排队，**不再在 vault TASKS 登记全量占用**。派出去的代理不编译、不测试、不交 testbox，命令写进回执交主会话交。
- 回到本机直跑（撤掉本节）：把 `bottega-local` 加回本机 `~/.claude/max-core/local-test-entry.json` 的 `allow_commands`、去掉脚本里的 `require_testbox`、改回本节，ADR-048 改「已废弃」。
