# Bottega

Max 自用的 macOS 原生 App：在本机聊天、管 Agent。Swift 6 + SwiftUI + Liquid Glass，Apple silicon、macOS 26 起。设计在 Studiolo 的 vault `项目/Studiolo-Docs/01-框架/Bottega/`（入口 `Bottega-入口.md`）。

**现在的状态**：代码是从 Tillerwright 原样搬来、只改了名（Studiolo T-225，2026-11-22，来源是冻结的本地仓库 `~/All-Project/1-Max_Workflow` 的 main `cd4cb8b`，历史留在那里）。

**P1（T-240）**：打开 App 默认是新界面，只当 LMAS 的窗口，经 `ui.sock` 直连本机的 LMAS，自己不起任何 CLI（协议在 LMAS 仓库 `docs/bottega-protocol.md`，版本 1）。Tillerwright 那套（自己起各家 CLI、Crew）还在，用启动参数 `-classic` 打开，两套互不引用；旧的那套整层去掉另开任务（vault Bottega-6 第 4 节）。下面「数据」里的项目脚本、「键位与外观」说的是 `-classic` 那套。

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
- `test` 跑 `BottegaTests`（不进 App，只看编出来的包）和核心包的 `BottegaKitTests`、`BottegaLinkTests`，都是 Swift Testing。只跑核心包：`scripts/bottega-local swift test --package-path BottegaKit`。
- 只编 Debug，Debug 用临时签名（ad hoc，不碰钥匙串）。**发布版由用户本人签**：脚本拒绝 Release 和 archive。
- `scripts/quick_run.sh` 是 Tillerwright 留下的「编完装进 `/Applications/Bottega Dev.app` 并重启」，会退出正在跑的那份，只在 Max 说「装上」时用。

## 连 LMAS

- Settings 里选 LMAS 的运行目录（里面有 `ui.sock` 的那个）。第一次连要登记：在 Secure Enclave 里做两把钥匙（日常、批准），批准那把每次签都要 Touch ID；一个 LMAS 只收一台设备，换钥匙要等 LMAS 片 8。每个 LMAS 各用一套钥匙；回执丢了或设备号丢了，用原来的钥匙再登记一遍拿回同一个号。
- 事件不落盘，每次连上从头取，断线只在内存里接着取。落盘的只有 Application Support 下 `lmas/`：钥匙的密文（Debug 版；发布版放钥匙串）、这台 Mac 在那个 LMAS 的设备号、还没发出去的话。
- 凡是要签名的（说话、答、批、拒、登记）只认前台窗口里真键盘、真鼠标的操作，别的程序发来的按键、点击、辅助功能按压都不认，一次按压只算一次，没有绕过的开关。
- 键位：Return 换行，⌘Return 发送；框空时 ⌘Return 打开等得最久的批准单，看完全文再按一次 ⌘Return 弹 Touch ID。J、K 在时间线、泳道、卡板里移动。⌘1～⌘5 切固定的五页。
- 批准单：App 连着、在跟事件，Mac 弹窗先让 2 分钟（LMAS 那边定的）；先点的算，后到的写「已经答过了」。签的摘要照 LMAS 发来的字节算，一个字节不改。
- 连真的 LMAS 试：在 LMAS 仓库里 `scripts/lmas-local demo`（假的 CLI，不花额度；`--dir <短路径>` 停了留着），Bottega 这边 `open --env BOTTEGA_RUN_DIRECTORY=<它打印的 ui.sock 所在的目录> "bottega/build.noindex/dev/Build/Products/Debug/Bottega Dev.app"`。
- 不用界面、不用指纹地验连接：`scripts/bottega-local swift run --package-path Probe bottega-probe <运行目录> <放钥匙的临时目录> [<把事件和一轮里的细节录成文件>]`，一行一个命令（`register <名字>`、`state`、`reconnect`、`reload`、`forget`、`seen`、`say <会话> <话>`、`interject`／`tell <线|last> <会话> <话>`、`answer <信|last> <话>`、`file <会话> <字节数>`、`asks`、`approve`／`decline <单|last>`、`approve-unread <单|last>`（带错的摘要）、`cut`（下一条意图的答复丢掉）、`flood <会话> <条数>`、`name <线> <标题>`、`pin`／`unpin`／`archive`／`unarchive <线>`、`read <线> [<号>]`、`add-project <名字> <路径>`、`remove-project <名字>`、`open <项目> <哪家> <角色> <模型|-> <强度|-> [<第一句话>]`、`model <线> <会话> <模型> [<强度>]`、`stop <轮>`、`lines`、`models`、`mail`、`overview`、`turns`、`turn <轮>`（取回来的和推来的对一遍，不一样就印出来）、`wait <毫秒>`、`quit`）。`Probe/` 是单独一个包，进不了 App；它登记的钥匙是软件做的，**只对演示用的 LMAS 用**。测试跑不了这件事：测试进程是 Apple 签的，LMAS 不认。
- LMAS 正式的 `--bottega` 不认带 `get-task-allow`、关了库校验之类 entitlements 的程序（协议第 2 节）。发布版编好后先看一眼：`codesign -d --entitlements - Bottega.app` 里没有这几样；有 `get-task-allow` 就在 `project.yml` 的 Release 里加 `CODE_SIGN_INJECT_BASE_ENTITLEMENTS: NO`。
- 只看界面、不连 LMAS：Debug 版 File › Open Recording…，开 `samples/waiting.jsonl`、`samples/turns.jsonl`（一轮里的细节，含 `live` 那些行）或 `BottegaKit/Tests/BottegaKitTests/Fixtures/events.jsonl`。只有 Debug 版认这几个环境变量：`BOTTEGA_RECORDING`（事件流文件）、`BOTTEGA_RUN_DIRECTORY`（这一次连哪个运行目录）、`BOTTEGA_PAGE`（`dispatcher`、`waiting`、`board`、`activity`、`devices`、`line:<号>`）、`BOTTEGA_DETAIL`（`card:<号>`、`message:`、`merge:`、`test:`、`turn:`）、`BOTTEGA_LANES=0`。
- 测试数据是 LMAS `fixtures/bottega/` 的复制品，不手改：`bottega/scripts/sync_lmas_fixtures.sh` 比对，`--apply` 更新。

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

新界面：

| 目录 | 内容 |
|---|---|
| `BottegaKit/Sources/BottegaKit` | 纯逻辑，不碰窗口和网络：线上类型、签名原文和验签、事件归约、发件箱、草稿发给谁、不可信文字的处理。P3 的 iPhone App 共用 |
| `BottegaKit/Sources/BottegaLink` | 连接：unix socket、握手、登记、跟事件、断线重连、发意图、Secure Enclave 的钥匙 |
| `Bottega/App/Workshop` | 界面：`Model` 是页面状态和唯一的发送口，`Views` 是各页 |

依赖只有一个方向：`BottegaKit` ← `BottegaLink` ← `Workshop`。`Workshop` 不引用下面那四层（只用 `Core/Support` 的 `AppInfo` 和 `HangWatchdog`）。

旧界面（`-classic`）：四层单向依赖，Core 不依赖任何上层，Services 与 UI 只依赖 Core，App 依赖全部：

| 目录 | 内容 |
|---|---|
| `Bottega/Core` | 模型与纯支持：项目与线程、时间线、provider 种类、Crew、快捷键、进程 I/O、JSON-RPC |
| `Bottega/Services` | Git、worktree、diff 解析、落盘 store、各 provider 适配器（统一事件模型）、MCP |
| `Bottega/UI` | 窗口 chrome、通用控件、Markdown 渲染、主题、音效 |
| `Bottega/App` | 入口、命令与设置、项目库、每线程 runtime、窗口、全部功能视图 |

新代码放依赖得起的最低层。
