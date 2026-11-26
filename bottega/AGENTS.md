# AGENTS.md — 在 Bottega 目录里干活的 agent 须知

Bottega 在 Studiolo 仓库的 `bottega/` 目录里。Studiolo 仓库根的 `AGENTS.md`、`CLAUDE.md` 和 Max 的全局 CLAUDE.md 照常适用；下面只写这个目录特有的。来源和署名见 `README.md`。

## 编译、测试（Mac Studio 本机的例外）

- **Bottega 在 Mac Studio 本机编译、测试，一律经仓库根的 `scripts/bottega-local`**（`generate`、`build`、`test`、`run`、`xcodebuild <参数…>`、`swift <参数…>`），不裸跑 `xcodebuild`、`swift`、`xcodegen`。依据是 Studiolo 的 vault ADR-043 和仓库根 `CLAUDE.md`「Bottega 在 Mac Studio 本机编译、测试」一节；MacBook 和别的机器上脚本自己拒绝，那时如实写「这台编不了」，不改交 108（108 编不了 macOS App）。
- 只编 Debug；发布版签名由 Max 本人做，脚本拒绝 Release。
- 派出去的代理（Workflow 代理、Crew hand）不编译、不测试，把命令写进回执交主会话跑。全量 `test` 同一时间只许一个会话跑，开跑前在 vault `04-协调/TASKS.md` 登记占用。
- `** BUILD SUCCEEDED **`、`** TEST SUCCEEDED **` 之前只能报「代码改完·未验证」。`xcodebuild` 日志尾部的 `(N failures)` 不含细节，`grep "error:"` 才看得到真正的错误。
- 改过 `project.yml` 或增删源文件不用手动 generate：`build`、`test`、`run` 每次都先生成工程。
- **编过不等于能跑**：`scripts/bottega-local run` 起来后确认进程还在，才算验收。启动即崩先看 `~/Library/Logs/DiagnosticReports/*.ips` 的 `termination` 段。

## 不要擅自重启装好的 App

`scripts/quick_run.sh` 会退出并重启 `/Applications/Bottega Dev.app`。只有 Max 明确说「装上 / 重启」才执行；平时验证用 `scripts/bottega-local run`（打开的是 `build.noindex` 里那份）。

## 分层

四层单向：Core ← Services / UI ← App。Core 文件若需要 provider 注册表、设置或视图，说明放错层了，拆成上层的 extension（范例 `App/CrewCookbook+Resolve.swift`）。

## 新界面（`BottegaKit`、`App/Workshop`）

- 依赖单向：`BottegaKit` ← `BottegaLink` ← `Workshop`。能做成纯逻辑的放 `BottegaKit` 并写测试；App 目标测不到，放进去的逻辑越少越好。`Workshop` 不引用旧的 `AppModel`、provider、Crew。
- 协议以 LMAS 的 `docs/bottega-protocol.md` 为准，和 `fixtures/bottega/` 对不上时以测试数据为准。这边不改协议，有问题找 LMAS 的会话。序号按事件编：接得上 = `seq` 等于上一条的 `through` 加 1。
- 别人写的字（信、卡、测试输出、会话名、`Why.text`）一律 `Text(verbatim: UntrustedText.clean(…))` 或 `.line(…)`，不渲染 Markdown、不做链接。批准单的 `text` 用 `UntrustedText.revealing(…)`：一字不删，看不见的字符写成 ⟨U+XXXX⟩；签的摘要按原文算。
- 要签名的动作只从 `WorkshopModel` 发，先过 `HumanInput`；不加环境变量、启动参数一类的绕过。
- 按当时情形定的意图（`IntentDraft.keeps` 为 false：批、拒、停一轮、换模型、加减项目）只在连上并追平时发：模型里先过 `canDecideNow()`，连接层 `LinkClient.submit` 没追平就当场抛 `ConnectionLost`，不留到重连以后。
- 线上来的数字（`UInt64`）不直接 `Int(x)`、不直接做加减：用来往下数的经 `decodeNumber`（上界 `Wire.farthest`，2^53），只拿来显示的在归约时 `min(…, Wire.farthest)`，界面里 `Int(clamping:)`。超了会当场崩。
- 不用指纹就能签的钥匙（`SoftwareKeys`）只在 `BottegaTestKeys` 里，给包的测试和 `bottega-probe` 用；`project.yml`、App 的源码、`BottegaKit`、`BottegaLink` 都不引用它（`TestKeysStayOutTests` 看着）。
- 红色只表示「等你」；失败用图标加橙色或次要色。界面文字英文。

## 几条本仓库的坑

- `ThreadTimeline.timelineScroll` 的 ViewBuilder 曾超出类型检查预算，已拆成 `earlierMessagesPill` / `blockStack` / `endSentinel`。**新增行放进子视图，不要往主 builder 里内联。**
- 上游源码是快照不是 git clone。碰到「某类型没有某成员」但用法遍布多处，先怀疑上游漏了提交，照消费端语义补全链路，不要删用法绕过。
- Debug 用临时签名、没有 Team ID：Debug 配置必须关 hardened runtime，否则 debug dylib 过不了 library validation。
- 所有落盘与 Keychain 走 `AppInfo.storageName` / `keychainService`，绝不走 `AppInfo.name`（那是显示名）。

## 新增一条命令 / 快捷键

1. `Core/Models/Shortcuts.swift`：`enum AppShortcut` 加 case，编译器会逼着补齐 `section` / `title` / `detail` / `defaultChord` 四个 switch（`defaultChord` 返回 `nil` 即出厂不带键）。rawValue 是用户改键记录（defaults 的 `keyboardShortcuts`）里的键名，发布后不改；非改不可就像 `finishThread = "archiveThread"` 那样钉住旧值。
2. `App/AppCommands.swift`：在对应的 `CommandMenu` / `CommandGroup` 里加 `Button(…) { … }.keyboardShortcut(shortcuts.keyboardShortcut(for: .新case))`。**不挂进菜单，快捷键就不响应。** 依赖选中线程的加 `.disabled(model.selectedThreadID == nil)`；要看线程状态读 `runtime?.isRunning`、`runtime?.draftIsEmpty` 这类只在翻转时变的值，别读 `draft`（每敲一个字都会重算整棵菜单）。
3. 可选，进命令面板：`App/Views/Palette/CommandPalette.swift` 的 `results` 加 `Item(…, shortcut: ShortcutStore.label(for: .新case)) { … }`。菜单与面板共用、不止一行的逻辑写成 `AppModel` 的方法，两边各调一行。
4. Settings › Shortcuts 按 `AppShortcut.Section` 自动列出，页面不用改（要新分组就给 `Section` 加 case）；录键冲突与保留键由 `ShortcutStore.refusal` 把关。说明文字里提键位用 `ShortcutStore.hint(for:)` / `label(for:)`，不写死「⌘J」，改键或清空后文案跟着变。

## 新增一个自用设置键

全在 `App/AppSettings.swift`，照现有的键写四处：

1. `private enum Key` 加 `static let fooBar = "fooBar"`（就是 defaults 里的键名，发布后不改）。
2. 属性，写回放 `didSet`：`var fooBar: Bool { didSet { defaults.set(fooBar, forKey: Key.fooBar) } }`。可选值为 nil 时 `removeObject`；Codable 值走 `store(_:forKey:)` / `Self.load(_:forKey:)`。
3. `init()` 读出并给默认：`fooBar = defaults.object(forKey: Key.fooBar) as? Bool ?? false`。用 `object(forKey:) as? T ?? 默认`，别用 `bool(forKey:)`：它分不清「没存过」和「存了 false」，默认为 true 的键会被读成 false。有取值范围的在这里夹紧（范例 `recentStepsShown`）。
4. UI 入口：对应设置页（`SettingsView.swift` 的 `GeneralSettingsPage`、`CrewSettingsPage.swift` 等）的 `ChromeCard` 里加 `ChromeRowDivider()` 与 `ChromeRow(title:detail:) { SettingsSwitch(isOn: $settings.fooBar) }`（页面 body 开头已有 `@Bindable var settings = model.settings`），再往该页的 `SettingsPage.keywords` 补搜索词。

加了键就要有读取点：App 层读 `model.settings.fooBar`；Services 经协议拿（范例 `Core/Models/ProviderSettings.swift`，`AppSettings` 遵从它）；Core 不碰设置，由上层以纯值传入。拿不到 `AppSettings` 的地方也要读的键（如新线程建出时），键名交给 `ChatThread` 以 `static let` 持有、`Key` 引用它，读取写成它的静态函数（范例 `ChatThread.crewDefaultKey` / `crewDefaultEnabled()`）。口令与 API key 走 Keychain（`APIKeychain`），不直接存 defaults。

## Crew hands

- hand 只改 brief 里点名的文件并报告，**不 build、不测、不跑任何检查**；所有 hand 交回后由主会话经 `scripts/bottega-local` 构建一次。
- delegation 块每个条目带 `files`，两个条目不得共用文件。

## 提交

中文，说清根因、改法与验证结论；同一根因的同类实例一并修全。
