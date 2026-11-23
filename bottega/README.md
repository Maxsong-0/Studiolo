# Bottega

Max 自用的 macOS 原生 App：在本机聊天、管 Agent。Swift 6 + SwiftUI + Liquid Glass，Apple silicon、macOS 26 起。设计在 Studiolo 的 vault `项目/Studiolo-Docs/01-框架/Bottega/`（入口 `Bottega-入口.md`）。

**现在的状态**：代码是从 Tillerwright 原样搬来、只改了名（Studiolo T-225，2026-11-22，来源是冻结的本地仓库 `~/All-Project/1-Max_Workflow` 的 main `cd4cb8b`，历史留在那里）。功能还是 Tillerwright 的：自己起各家 CLI、Crew 多 agent。P1 起改成只当界面、直连本机的 LMAS（vault `Bottega-7-分期.md`），provider 层整层去掉。

## 出处

Bottega 是 [Droppy Code](https://getdroppycode.app) 1.6.0 的修改版（经 Tillerwright 魔改，再改名重做）。**Droppy Code by Jordy Spruit (Droppy), https://getdroppycode.app**，以 GNU AGPL v3 加署名附加条款发布，它本身源自 T3 Tools Inc. 的 T3 Code（MIT）。终端是 Miguel de Icaza 的 SwiftTerm（MIT）。

这是一个**修改版**，不是 Droppy Code，也不由 Droppy 或 Jordy Spruit 发布、背书。只自用、不分发、不给别人经网络用（vault Bottega-6 第 4 节）。

照 [LICENSE](LICENSE) 第 7(a) 条，署名放在这几处：About 页和 App 的版权字段、本 README、每个源文件的第一行（或 shebang 下一行）。第三方声明 [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) 和商标政策 [TRADEMARK.md](TRADEMARK.md) 取自 Droppy Code 1.6.0 原样（Tillerwright 曾按它的 ADR-003 删掉，搬进 Studiolo 时补回），和 LICENSE 一起打进 App、在 Settings › About › Licenses 里看得到。新加的源文件也照第一行的写法带上署名。[CHANGELOG.md](CHANGELOG.md) 是上游 Droppy Code 的。

## 编译、测试、运行

**只在 Mac Studio 上，一律经仓库根的 `scripts/bottega-local`**（Studiolo 的 vault ADR-043；别的机器上它自己拒绝）。不裸跑 `xcodebuild`、`swift`：

```bash
scripts/bottega-local build
```

```bash
scripts/bottega-local test
```

```bash
scripts/bottega-local run
```

- `build`／`test`／`run` 先按 `project.yml` 用 xcodegen 重新生成 `Bottega.xcodeproj`（**不要手改工程**），再编 Debug 版 `Bottega Dev.app`，产物在 `bottega/build.noindex/dev`（每个 worktree 一份）。`run` 编完直接打开它，不装进 `/Applications`。
- `test` 跑 `BottegaTests`（Swift Testing，宿主是 Debug 版 App）。
- 只编 Debug，Debug 用临时签名（ad hoc，不碰钥匙串）。**发布版由用户本人签**：脚本拒绝 Release 和 archive。
- `scripts/quick_run.sh` 是 Tillerwright 留下的「编完装进 `/Applications/Bottega Dev.app` 并重启」，会退出正在跑的那份，只在 Max 说「装上」时用。

## 数据

Debug 与 Release、以及本机若装有上游 Droppy Code，三者的数据完全分开：

| build | Application Support | 家目录 | bundle id |
|---|---|---|---|
| Bottega Dev | `Bottega Dev/` | `~/.bottega-dev/` | `me.studiolo.bottega.dev` |
| Bottega | `Bottega/` | `~/.bottega/` | `me.studiolo.bottega` |
| 上游 Droppy Code | `Droppy Code/` | `~/.droppy-code/` | `iordv.droppycode` |

想把上游的项目与线程带过来：`scripts/import_from_upstream.sh`（默认 dry run，`--apply` 才动手，不导 Keychain）。

每个项目根目录可放 `bottega.json` 定义项目脚本；上游留下的 `droppy-code.json` 也认。全局脚本（每个项目的每个线程都能跑）放在 Application Support 下的 `Bottega/bottega.json`（Dev 版在 `Bottega Dev/`），格式相同，也可从 ▶ 菜单或顶层菜单 Workflow 打开编辑器改：

```json
{ "scripts": [ { "name": "Build", "command": "scripts/quick_run.sh", "icon": "hammer.fill", "runOnWorktreeCreate": false } ] }
```

- 两份文件改了即生效：切到线程、回到 app、打开 ▶ 菜单、运行脚本前按修改时间重读。项目文件里列的脚本以文件为准，在项目脚本编辑器里只读；全局脚本编辑器保存时直接写回全局文件。
- `icon` 写任意 SF Symbol 名（旧的 test / lint / configure / build / debug 仍认），本机没有的名字回退 ▶。
- Workflow 菜单列出全局脚本与项目脚本，前九个依次是 ⌥⌘1–9；全局脚本排在前面，所以同一个全局脚本在每个项目里键位不变。
- 同一脚本再次运行会在它上次的终端 tab 里重新开始，不再每次新开 tab。
- 脚本里 `$BOTTEGA_PROJECT_ROOT`（兼容 `$DROPPY_CODE_PROJECT_ROOT`）是线程的工作目录：worktree，或项目根。

## 键位与外观

- chat 框：Return 换行，⌘Return 发送（回合运行中＝打断并发送；框空时＝批准顶部的请求），⇧⌘Return 排到当前回合之后。Esc 只收面板，不再停回合；停回合用 ⌘.，或 Thread › Stop and take back。⌘Return 是固定键，不能录给别的命令。
- 其余快捷键在 Settings › Shortcuts 改。新增：⌥⌘.（停止本聊天全部 hand）、⌥⌘←（折叠全部轮次）、⌥⌘T（显示/隐藏思考）、⇧⌘F（搜索线程）、⌃⌘T / ⌃⌘L（下一个主题 / 深浅切换）。
- `/sop`、`/sop-close` 把并发 SOP 的开工 / 收尾清单发给 lead，命令面板和 Workflow › Prompts 里也有。
- 内置主题只有 Bottega、System、Light、Dark 四套。自定义主题写在 Application Support 下的 `theme/theme.json`，Settings › General 的 Theme file 行可一键生成模板；保存即生效，写错的键会在设置页报出路径。颜色可写 `#RRGGBB`、`#RGB` 或带透明度的 `#RRGGBBAA`，`glass` 块调各玻璃表面的强度。
- Settings › General 的 Text size 缩放整个窗口（70%–160%），正文字体、代码字体和终端字号可以分别设。

## 代码分层

四层单向依赖，Core 不依赖任何上层，Services 与 UI 只依赖 Core，App 依赖全部：

| 目录 | 内容 |
|---|---|
| `Bottega/Core` | 模型与纯支持：项目与线程、时间线、provider 种类、Crew、快捷键、进程 I/O、JSON-RPC |
| `Bottega/Services` | Git、worktree、diff 解析、落盘 store、各 provider 适配器（统一事件模型）、MCP |
| `Bottega/UI` | 窗口 chrome、通用控件、Markdown 渲染、主题、音效 |
| `Bottega/App` | 入口、命令与设置、项目库、每线程 runtime、窗口、全部功能视图 |

新代码放依赖得起的最低层。
