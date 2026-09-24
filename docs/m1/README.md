# M1 本机只读工作台

基线为已验收的 M0 `0bd066d`。本目录只记录实现约定，完整里程碑计划在 Obsidian 的 `项目/Studiolo/04-协调/2026-09-04-M1-实施计划.md`。

**读之前先知道边界（2026-09-17 核实）**：`studiolo-local-server` 这个二进制早已不只承担 M1。
它按启动参数分别装配 M1 只读派生视图、M2 会话认证 / canonical 读取 / canonical 写者 / 人侧编辑与
文件操作 / 健康页，以及 M3 图 2 的仓库绑定。**本文件只描述 M1 的只读入口**（只给 `--vault`、
`--assets`、`--label`、`--port` 时的形态）；其余模式的契约、状态与未完项在 `docs/m2/`、`docs/m3/`，
当前总验收台账是 [M2、M3 整体收尾](../m2/m2-m3-closeout.md)。参数全集以
`studiolo-local-server --help` 的实际输出为准。

## 已实现切片与后续

**T-006 + T-007 + T-010 + T-011 已实现，完整 M1 尚未完成。** 基础链路为：本地显式副本 → Broker 独立
只读 Adapter → WorkbenchReadModel → loopback HTTP。

- T-006：项目总览、待我动作、文档原文与行号定位，支持筛选与手动全量刷新。
- T-007：中文全文字面搜索、精确项目筛选、完整路径片段快速切换，以及同代次大纲、
  wikilink 出链与反链导航。详见 [搜索与文档导航契约](search-navigation.md)。
- T-010：同代次的文件树、空目录与纯附件目录、按需分页、只读附件信息。
  入口「文档 → 文件树」；详见 [文件树契约与验收](file-tree.md)。
- T-011：Apple 浅色材质、统一语义图标、可打断动效和双尺寸布局；保留全部只读能力。
  当前视觉规范以 [T-011 设计系统](design/t011-design-system.md) 为准，替代下方历史 T-006 视觉约束。
- T-035：在 T-011 基础上完善界面与动效并合并 main（`49a9f1a`），由用户手动把服务器 4319
  网页服务切到该运行包；范围、证据与限制见 [T-035 发布记录](ui-release.md)。
  **该记录里的公网登录后交互验收仍是未完项**，不能拿「已切换」当 M1 验收。

尚未包含：附件预览/下载、变更流、规则漂移、持久索引与 watcher。源目录始终零写入，
不观察或自动刷新源目录，前端不调用 M0 写引擎。

## 启动纯虚构示例库

在仓库根目录运行：

```bash
npm --prefix web ci --ignore-scripts
npm --prefix web run build
cargo run --locked -p studiolo-local-server -- --vault examples/m1-vault --assets web/dist --label 示例库 --port 4317
```

访问 [http://127.0.0.1:4317/](http://127.0.0.1:4317/)，按 Ctrl-C 停止。
Host 校验只接受该 IPv4 loopback 地址与端口，不能改用 `localhost`。

用户已指定服务器运行示例版，部署进度、完整校验与访问方式见
[T-008 服务器运行说明](server-preview.md)；它使用服务器回环加 SSH，不开放 LAN/公网 HTTP。
按用户后续指令部署 [T-009 域名入口](cloudflare-preview.md)：单邮箱 Access、独立
4319 鉴权服务、Tunnel 和域名路由已生效，待真实邮箱登录及公网功能验收；原回环服务不变。

`examples/m1-vault` 全部是人工虚构数据，不含真实用户笔记。入口不自动读取或复制真实
Vault，也不自动创建副本。真实内容只能由用户明确选择一个只读副本，再通过 `--vault`
传入；`--label` 仅控制界面显示名称，不能改变读取权限。`--assets` 应指向构建后的
`web/dist`，不能用笔记目录代替。原文只显示转义文本，没有编辑、任务执行或真实 Runner。

## T-011 前端验证（2026-09-04 历史记录）

**这是 T-011 当时的数字，不是当前前端状态**：此后 M2 的会话、因子、编辑、历史、文件操作与
Rename 界面又新增了大量文件与用例，当前口径见 [收尾台账](../m2/m2-m3-closeout.md)。

60 项测试、lint/format/类型检查/生产构建通过。CUA IAB 在独立 [4321 预览](http://127.0.0.1:4321/)
验证1536×1024和390×844的筛选、搜索、目录与附件、原文导航/历史、快速切换/键盘/焦点和刷新。
原生尺寸概念与最新截图同轮 view_image 对照；无重大未处理视觉偏差，字级/图标/对比度调整已记录。
控制台无warn/error；虚构源内容哈希与目录清单不变。本轮未改后端、未部署或提交推送，
真实iPhone/Safari/辅助偏好系统切换及公网登录未验收；详见设计规范中的证据与边界。

## T-010 后端及历史前端验证

macOS 222 / Linux aarch64 230 项 Rust 测试通过，fmt 与严格 Clippy 通过；Linux 路径安全
8 项另行复跑通过，不重复计数。前端 39 项、lint/Prettier/类型检查/生产构建通过。
1536×1024 与 390×844 经独立 Rust 4320 + 生产资源，目录/空态/附件信息/分页/原文/刷新/
ESC 焦点通过；无控制台警告错误、外部资源或附件内容请求，源内容与目录清单不变。
本片未部署服务器；不使用旧版服务器验收或本机浏览器结果冒充新版公网通过。

## T-006 + T-007 历史验证记录

截至 2026-09-04，分支为 `codex/m1-readonly-workbench`，尚未提交推送；M1 远端 CI
尚未触发。已实际通过：

- macOS：158 项 Rust 测试；格式与 Clippy 检查通过。
- Linux：166 项 Rust 测试；格式与 Clippy 检查通过。
- 前端：28 项测试；lint、Prettier 与生产构建通过。

后续 T-009 域名鉴权代码在 macOS 已通过 189 项 Rust 测试与格式/Clippy 检查；
新包服务器原生 197 项、前端 28 项及生产构建、17 项实际 HTTP 检查通过。
真实邮箱登录后的公网功能验收仍待用户配合，详见 [域名入口进度](cloudflare-preview.md)。

本轮以生产构建配合真实 Rust 同源 HTTP，在 1536×1024 桌面和 390×844 窄屏完成：

- “连续阅读”跨项目命中；收窄到纸页仅保留该项目，打开原文第 6 行。
- 大纲与 wikilink 标题定位到第 8 行，同名配色方案展示完整路径并可选择；后退恢复原位置，反链打开引用第 10 行。
- 缺页/缺标题分别提示；代码围栏伪链接不列入导航，原文脚本转义显示。
- ⌘K 打开快速切换，路径多片段匹配；方向键选择、Enter 打开、Esc 关闭恢复焦点。
- 搜索空态和手动刷新正常；刷新清空旧查询与来源历史。旧代 409、槽满 429、请求竞态与取消额度由自动化测试补充覆盖。
- 窄屏导航文字单行、搜索和原文无整页横向溢出；原文导航区域局部滚动。控制台 warn/error 为空。

视觉检查确认新搜索控件沿用既有字级、配色、间距和开放列表布局；原文侧栏增加限高导航区，
窄屏上下排列并保留全部入口。这些属于功能扩展，不声称原概念图已经覆盖新界面。
截图为本机 `m1-search.png`、`m1-navigation.png`、`m1-search-mobile.png`、`m1-navigation-mobile.png`
验收产物，不入仓库。所有本轮夹具和交互验收仅使用虚构库，不读取真实用户笔记；未做真实 Vault 性能基准。

## T-006 历史验证与视觉验收

以下为第一片完成时的记录，保留历史语义，**不是含 T-007 的当前测试总数**：

- macOS：124 项 Rust 测试。
- Linux：132 项 Rust 测试；格式与 Clippy 检查通过。
- 前端：13 项测试；生产构建、lint、Prettier 检查通过。

T-006 当时完成生产构建经 Rust 同源真实 HTTP 的浏览器验收，覆盖 1536×1024 桌面与
390×844 窄屏：

- 项目 / 生命周期筛选、无匹配空态、三导航、手动刷新可操作。
- 待重启动作可定位到 `TASKS.md` 第 19 行；ESC 关闭原文后焦点回到触发按钮，窄屏原文查看可用。
- 控制台无 warn / error。窄屏仅表格自身横向滚动；文档宽 375px，小于视口 390px，无整页横向溢出。

概念图与最终桌面、窄屏截图已在同一轮对照：布局、字级、配色、图标、间距五项保留。
有意差异为：全部已知项目来源可点击；缺失值显示“未知”；未建档用 profile 标签而非
虚构生命周期；真实动作正文保留等待前缀；页脚之后补充折叠诊断。最终截图为本机验收
产物，不复制进仓库。

设计对照账（最终桌面截图与概念图同轮检查）：

| 核对点 | 参考与实际 | 处理 |
|---|---|---|
| 布局 | 约 276px 侧栏、左侧导航、右侧表格和动作列表；实际保留相同层级与列结构 | 保留开放布局，未加卡片外壳 |
| 字级与控件 | 参考主标题约 42px、分节 32px、控件约 20px；实际显式使用同档字级 | 没有退回浏览器默认小号控件字 |
| 配色 | 白底、冷浅灰侧栏、青绿强调；初版强调色略偏蓝 | 最后一轮将共享颜色 token 对齐下方规格，保持纯白背景 |
| 图标 | 总览四格、方框勾、文档、锁、文件夹、搜索、刷新、下箭头 | 同隐喻的等线宽 outline SVG，文字与控件仍由代码渲染 |
| 间距与容器 | 参考表格后接三行收件箱、底部只读声明；初版诊断占据声明之前的位置 | 将诊断移到声明后折叠，保留分隔线与留白；真实较长文案允许换行 |

视觉收口结论：首片主屏在上述五项满足交付要求，但不宣称逐像素相同；浏览器滚动条、
系统字体抗锯齿与真实数据文案仍会带来细微差别。窄屏为同系统的响应式延伸，不另称已匹配一张不存在的窄屏概念图。

这一组历史结果仅验收 T-006 首片，不代表 T-007 或完整 M1 的退出验收完成。M0 的历史
测试和 CI 证据仍保留在 `docs/contracts/`，不与当前或 T-006 的数字混写。

## 运行契约

- 不提供默认真实 Vault 路径，也不自动复制用户笔记。
- 首片使用整批只读扫描，显式刷新时原子替换 generation；出现不可读文件、非法 UTF-8 或预算超限时不发布部分视图。
- 搜索和原文 / 导航请求必须携带当前 generation；刷新之后旧位置返回 409，避免拿新内容解释旧行号。
- 搜索与原文 / 导航共用两个后台工作槽；满时返回 429，客户端取消不提前释放仍在运行的工作槽。
  首屏 `/api/workbench` 同样占这两个工作槽，但满额时**排队等待**而不是 429：它是整个界面的
  入口，没有可退回的上一屏（T-061，2026-09-17）。
- 并发 `?refresh=true` 合流：任一时刻至多一次派生在途、至多一次排队，因此任意数量的并发刷新
  最多换来两次重扫，而不是每个请求各跑一遍；合流进同一张票的请求读到同一个结论。派生自己
  负责结算，客户端取消不会留下无人结算的在途刷新；运行者死亡（panic 或任务被取消）时闸门被
  显式释放，覆盖到的请求收到 503 并说明原因，排队中的那次刷新由等待者接手重跑（T-061）。
- Host 仅接受 `127.0.0.1:<端口>`；同源 Origin 与 Fetch Metadata 检查；无 CORS。应用只绑定 IPv4 loopback。默认本机模式不适用于公网；T-009 显式 Access 模式的鉴权与隧道要求见域名入口说明。
- 内容只显示为转义纯文本，不执行 Markdown 中 HTML、脚本或远程资源。
- 静态文件来自显式 `web/dist` 只读根；仅 `/` 与 `assets/` 下 js/css/svg/woff2 可读。笔记不通过静态路径暴露。
- **M1 只读入口**（只给 `--vault` / `--assets` / `--label` / `--port`）不装配 FsVaultBroker、
  M0Workbench 或 TaskEngine；源目录不会被初始化为 canonical Vault，源目录始终零写入。
  2026-09-17 核实：`FsVaultBroker` 只在显式的 `--canonical-writer-data` 写者模式下装配，
  `--canonical` 只读模式另走窄能力读取，两者都要求会话配置，不能从 M1 模式隐式升级；
  其边界见 [canonical 读取契约](../contracts/canonical-read.md) 与
  [canonical 单写者契约](../contracts/canonical-writer.md)。M0Workbench 与 TaskEngine
  在任何 local-server 模式下都不装配。

## Interface

以下是 M1 只读入口的四个端点；2026-09-17 按 `crates/local-server` 源码核实，并用上面那条启动
命令实跑 `127.0.0.1:4317` 逐个请求过：五条路由均 200，旧代次实测 409，`/api/v1/health` 在 M1
模式下实测 404（未挂载）。**同日 T-061 改掉了首屏形状**：`/api/workbench` 顶层字段现为
`generation / source_label / projects / inbox / counts / diagnostics`（M1 模式确实不产生
`freshness`），逐篇 `documents` 清单已去掉，改由 `counts` 给计数——这一条不是上面那次实跑的
结论，而是由 `crates/local-server/tests/workbench_overview_contract.rs` 按响应体逐字段断言。
同一路由表在其它模式下还会挂上会话（`/api/v1/sessions` 等）、canonical（`/api/v1/file`）、
编辑（`/api/v1/mutations`）、历史（`/api/v1/history`）、文件操作（`/api/v1/file-operations`）、
健康（`/api/v1/health`）端点，它们不属于 M1，契约分别在 `docs/contracts/` 与 `docs/m2/`、`docs/m3/`。

**两条例外（2026-09-17 T-056 按 `crates/local-server/src/routing.rs:223-232` 核实）**：
`GET /api/v1/bootstrap`（`bootstrap.rs:8`）与 M3 图 2 的 `GET /api/project-operations`
（`project_operations.rs:38`）挂在**任何模式检查之前**，因此 M1 只读入口同样可达，
不是「只在其它模式下才挂上」。bootstrap 只回 `{"authentication":"local|access|session"}`，
M1 模式固定为 `local`，不含身份、Cookie、CSRF 或配置；project-operations 在未用
`--project-repository` 绑定仓库时只投影当前只读快照内的项目结构，`repository` 段显示未核验，
不读取任何仓库目录。二者契约见 [浏览器会话入口](../contracts/frontend-session.md)
与 `docs/m3/project-operations.md`。

- `GET /api/workbench`：`{generation,source_label,projects,inbox,counts,diagnostics}`；
  canonical 模式下额外带可选 `freshness`，M1 只读模式不产生该字段。
  `counts` 为 `{documents,projects,inbox,diagnostics,active_tasks,open_bugs}`，每项都是
  `{value,unavailable}`：`value` 为 null 表示该计数不可得，`unavailable` 逐条说明是哪个来源
  让它不可得，绝不退化成 0。响应规模只随项目数与收件箱条目数增长，不随文档数增长；逐篇文档
  走 `GET /api/tree` 的分页与 `GET /api/search`，首屏不再整份下发。
- `GET /api/workbench?refresh=true`：重扫只读副本并发布完整代次；不写源文件。
  并发刷新合流（见上「运行契约」），刷新失败返回 422 并保留上一代完整视图。
- `GET /api/search?q=<字面查询>&mode=full_text|path&project=<可选项目ID>&generation=<代次>`：`{generation,hits:[{path,title,line,snippet,kind}],truncated}`。
- `GET /api/document?path=<URL encoded logical path>&generation=<number>`：`{path,title,content,navigation:{outline,outgoing,backlinks,truncated}}`。
- `GET /api/tree?path=<URL encoded directory or empty root>&offset=0&generation=<number>`：每目录最多 200 项；详见 [文件树 Interface](file-tree.md)。

聚合字段为 `{value,source}`，未知用 null；source 为 `{path,line}`（一基行号）。派生行不得靠最后修改时间推断生命周期；解析规则、原文与 M0 索引在纯 workbench Module 内集中。
搜索模式、命中上限、导航歧义和缺失状态详见 [T-007 Interface](search-navigation.md)。

## T-006 历史视觉规格（已被 T-011 取代）

第一片的完整主屏参考为 `design/overview-concept.png`（1536×1024）；使用内置图像生成工具，以纯虚构示例项目生成，没有上传真实笔记。
T-007 新增搜索、快速切换和文档导航沿用该设计系统，但须独立验收，不能把原概念图
当作已经包含新增交互的证明。

Prompt 摘要：中文本机只读工作台；Studiolo 文本品牌；项目总览/待我动作/文档三导航；筛选与刷新；四行项目表、三行待我动作；白色主面、浅灰侧栏、青绿强调；禁止营销卡片、额外指标、云功能与编辑动作；界面必须 HTML/CSS 实现。

- 原图白底 `#ffffff`、侧栏近 `#f7f8fa`、文字 `#20252b`、辅助 `#747c86`、细线 `#e8ebef`、强调青绿约 `#087f72`。
- 原生尺寸侧栏约 276px，内容左沿约331px，右边距48px；表格/列表以分隔线组织，无卡片外壳。
- 系统中文无衬线；品牌32px、主标题约42px、分节32px、正文/控件约20px（1440与更窄屏等比/分档收敛，不压成不可读文字）。
- 图标仅用等线宽 outline：四格总览、方框勾收件箱、文档、锁、文件夹、搜索、刷新、下箭头。
- 首屏固定文案：项目总览；项目状态与待办，回到原文核对。；刷新；筛选项目；全部状态；项目/生命周期/当前版本/进行中/未关闭 BUG/下一里程碑；待我动作/查看全部/查看原文；本机只读；示例库；所有内容来自只读副本；不会修改笔记。
- 原文侧栏、错误/空状态、非标准项目诊断是必要交互扩展，沿用同一系统；动态值来自后端夹具，不以概念图假数据代替读取结果。
- 初版对源字段使用可点击回查样式；所有有效字段可追溯，未知值不伪造为零。窄屏保留完整信息，不能仅隐藏必要列。
