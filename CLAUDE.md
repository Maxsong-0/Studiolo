# Studiolo 仓库规则

@AGENTS.md

权威优先级与并发协作 SOP 见全局 `~/.claude/CLAUDE.md`；项目档案在 Obsidian vault `项目/Studiolo-Docs/`（先读 `00-总览/INDEX.md` 与 `04-协调/TASKS.md`）。

## Bottega 在 Mac Studio 本机编译、测试（全局「Mac 上不编译」的例外，用户 2026-11-22 定，vault ADR-043）

- **范围只有三样**：Bottega（仓库 `bottega/`，原生 Mac App）、以后的 Bottega 桥（Studiolo 在 Mac 上的常驻进程），和 P3 的 iPhone App（同一个 Swift 核心包，免费 Apple ID 装的 7 天重签也在这台重编重装）。它们要验的东西只在 Mac 上有（Secure Enclave、钥匙串、Touch ID、`LOCAL_PEERTOKEN`、代码签名的指定要求），108 编不了也跑不了。**网页、169 那一侧的代码照旧交 108**（`AGENTS.md` 第四、五节），合并门禁照旧是测试箱的 ci 套。
- **只限 Mac Studio 这一台**：MacBook 上没有 LMAS，端到端测不了；在别的机器上如实写「这台编不了」，不改交 108。
- **一律经 `scripts/bottega-local`**（`generate`、`build`、`test`、`run`、`xcodebuild <参数…>`、`swift <参数…>`），不裸跑 `xcodebuild`、`swift`、`xcodegen`。脚本自己只认 Mac Studio，别的机器上拒绝。命令名 `bottega-local` 登记在本机 `~/.claude/max-core/local-test-entry.json` 的 `allow_commands` 里（同目录 `local-test-entry.md` 有一节）：被拦了先查这条登记还在不在，不在就告诉用户，不要自己改登记。
- **只编 Debug**：发布版的签名照旧由用户本人做，脚本拒绝 Release 和 archive；本机编出来的开发版和测试拿不到发布版的钥匙串条目。测试不碰真的 LMAS 运行目录和真的钥匙（起测试用的 LMAS，或者用假的 socket）。
- 全量 `scripts/bottega-local test` 同一时间只许一个会话跑，开跑前在 vault `04-协调/TASKS.md` 登记占用。派出去的代理不编译、不测试，命令写进回执交主会话跑。
- 撤掉这条例外：把 `bottega-local` 从 `allow_commands` 里删掉、删掉 `local-test-entry.md` 那一节、删掉本节，ADR-043 改「已废弃」。
