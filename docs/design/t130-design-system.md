# T-130 · 瑞士网格＋工程图纸设计规范

2026-09-22 用户拍板。视觉定稿是画布「Studiolo 视觉方向样品」第三轮的**主稿**
（<https://claude.ai/artifact/6gYLEPt9ScusCqp1fuTZ2t>，页「第三轮 融合」：驾驶舱、协作页、⌘K 面板三张可点稿）。
本规范取代 [T-011 Apple 通透工作台](../m1/design/t011-design-system.md)。实现时有歧义，以本文为准，其次看画布；两者都没写到的，
取「更朴素、更少颜色、更少动效」的那一边，并在回执「学到什么」里写明。

一句话：**A2 管骨架，A4 管表达。** 黑色细线分区、巨号数字、只有一个红色（A2）；协作画成拓扑图、待办用圈号标在图上、
系统读数进图纸标题栏（A4）。

## 1. 令牌（`web/src/styles/tokens.css` 单一来源）

所有样式只引用令牌；深色值只在 `@media (prefers-color-scheme: dark)` 里重定义同名令牌（沿用 T-062 的做法，禁止局部深色补丁）。

| 令牌 | 浅色 | 深色 | 用途 |
|---|---|---|---|
| `--paper` | `#F1F1EC` | `#121211` | 页面与格子底色 |
| `--paper-raised` | `#F7F7F3` | `#1A1A18` | 表格行、输入框底 |
| `--ink` | `#0E0E0E` | `#ECECE6` | 正文、分区线、选中块 |
| `--ink-2` | `#55554F` | `#A6A69E` | 次要文字、标签 |
| `--ink-3` | `#8A8A83` | `#75756E` | 休眠、禁用、坐标刻度 |
| `--hairline` | `#CFCFC8` | `#34342F` | 格子内部的行分隔 |
| `--grid` | `#E3E3DC` | `#1E1E1B` | 网格纸细线（20px） |
| `--signal` | `#E61919` | `#FF4A3D` | **唯一强调色**：只表示「等你」或「出问题」 |
| `--signal-text` | `#C81414` | `#FF6A5F` | **14px 及以下的红色文字**（纸色上 5.2:1）；形状、大字、边框仍用 `--signal` |
| `--signal-wash` | `#FBE3E1` | `#3A1714` | 红色项的底（冲突路径框） |
| `--on-ink` | `#F1F1EC` | `#121211` | 黑底块上的字 |

- **不再有**蓝色强调、渐变、毛玻璃、柔影。选中＝黑底反白；悬停＝`--paper-raised` 底。
- 状态不靠颜色单独表达：`■` 运行/正常（ink 实心）、`□` 进行中/待定（ink 空心）、红 `■` 故障/等你、灰虚线框 休眠。
- 阴影只有一种：浮层 `box-shadow: 10px 10px 0 var(--ink)`（硬偏移，无模糊）。
- 圆角一律 0，唯一例外是圈号（圆）。
- z 轴只用四档：`--z-sticky: 10`、`--z-popover: 20`、`--z-dialog: 30`、`--z-toast: 40`。

## 2. 字体与字号

- 拉丁与数字：**Archivo**（500/700/800/900）；等宽标签：**IBM Plex Mono**（400/500/600）。
  用 `@fontsource/archivo`、`@fontsource/ibm-plex-mono` 自托管（CSP 是 `font-src 'self'`，禁止 Google Fonts）。中文走系统 `PingFang SC`。
- `font-family: 'Archivo', 'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', sans-serif;` 等宽：`'IBM Plex Mono', ui-monospace, monospace;`
- 数字一律 `font-variant-numeric: tabular-nums`。

| 角色 | 字号/行高/字重 | 例 |
|---|---|---|
| 巨号 | 196px / 0.78 / 900，字距 -0.07em | 驾驶舱「05」 |
| 大号读数 | 72px / 0.85 / 900，字距 -0.05em | 移动端巨号、统计 |
| 图名 | 34px / 1 / 900，字距 -0.03em | 标题栏「驾驶舱」 |
| 格子标题 | 18px / 1.3 / 800 | 「待我动作」「协作拓扑，今天」 |
| 条目标题 | 15.5px / 1.4 / 700 | 待办标题 |
| 正文 | 14px / 1.6 / 400 | 说明文字，最宽 65ch |
| 等宽标签 | 11–12px / 1.3 / 500–600，字距 0.04–0.08em | 「最久 5 天」「T-130」 |

品牌字标：`STUDIOLO` 21px/900/字距 -0.03em，后接红色句点。

## 3. 网格语法

- 页面外边距 20px 28px 24px。顶栏 46px，底边 3px `--ink`。
- 内容区是 **12 列网格，格子之间 1px 黑线**：容器 `background: var(--ink); gap: 1px; border: 1px solid var(--ink)`，格子 `background: var(--paper)`。
  这是分区的唯一手段；格子内部行分隔用 `--hairline`，不再套边框卡片。
- 格子标题行：左 18px/800 标题，右等宽 11.5px 次要信息或文字链接。格子内边距 14px 20px。
- **网格纸**只用于「图」：拓扑、泳道等画图区域的底，`background-image` 两层 1px `--grid` 线、间距 20px。文字区不用网格纸。
- **标题栏**（A4）：驾驶舱右下角，第一列跨三行放图名，其余格子「等宽小标签＋15px 等宽值」。
- 巨号只在每页出现一次；每页至多一个大号读数区。

## 4. 信息架构

顶栏五个导航，顺序固定：**驾驶舱 · 项目 · 文档 · 协作 · 策略**。当前项＝黑底反白块。

| 视图 `view` | 面板 `panel` | 内容 | 旧视图来源 |
|---|---|---|---|
| `cockpit` | — | 驾驶舱（第 6 节） | `overview`＋`inbox`＋健康 |
| `projects` | `list`（默认）/ `operations` | 项目明细表；选一个项目进入「运作」：结构声明、目录漂移、架构、Cargo 依赖、Rust 调用 | `overview` 表格＋`operations` |
| `documents` | `search`（默认）/ `tree` | 搜索、目录树、阅读器、编辑器、草稿、历史、文件操作 | `documents` |
| `collab` | `topology`（默认）/ `board` / `timeline` / `execution` | 协作拓扑；任务与回执（含本机派发）；时间线回放；执行与候选产物 | `board`＋`execution` |
| `policy` | — | 策略平面 | `policy` |

**旧链接必须继续可用**（`viewLocation.ts` 解析时迁移，`formatViewLocation` 只写新名）：

| 旧 hash | 新位置 |
|---|---|
| `view=overview` / 空 | `view=cockpit` |
| `view=inbox` | `view=cockpit`（落到待我动作格子） |
| `view=operations&project=P&panel=X` | `view=projects&panel=operations&project=P`（旧 `panel` 值原样放进 `item`） |
| `view=board&project=P` | `view=collab&panel=board&project=P` |
| `view=board&panel=replay&project=P` | `view=collab&panel=timeline&project=P` |
| `view=execution&project=P&item=I` | `view=collab&panel=execution&project=P&item=I` |

BUG-015（切走再回来回落首页）的修法——位置写进 hash、重挂后回到原处——必须保留，新增视图同样适用。

## 5. 外壳

- 顶栏左：字标＋五个导航。顶栏右：**状态胶囊**＋**⌘K 入口**（250px 等宽描边框「⌘K 跳转、搜索或执行动作」）＋账户入口＋内存草稿计数（有草稿时）。
- **状态胶囊**把新鲜度、本机健康、会话三件事收成一个（A 轮先做前两件与数据提示，会话一项在 C 轮接入）：全部正常显示 `■ 正常`；否则显示最严重一项，如红色 `[ 同步偏慢 ]`。
  点开是一个浮层，逐项列出三块的原文与「重新核验」动作（原 `FreshnessNotice`、`HealthPanel` 的内容挪进来，一字不少）。
  **不再在每页顶部挂新鲜度横幅，不再在每页底部挂「数据提示」**：诊断信息挪到各自所属的格子里，或进胶囊浮层。
- 窄屏（<760px）：顶栏只留字标、**当前库名**、胶囊和 ⌘K 图标；底部固定五格导航（44px 触控高度）；12 列网格收成单列，巨号降为 72px。
- **当前库名**（工作台 `source_label`）在任何宽度下都显示在字标右侧（等宽 11.5px，超长省略但 `title` 给全名），用户始终知道在看哪个库（承接 T-117）。
- **项目入口不许静默截断**：凡是只显示前 N 个项目的地方（驾驶舱项目明细、⌘K 项目组），必须同时给出「全部 N 个项目」入口并写明总数（承接 T-117）。

## 6. 驾驶舱

画布主稿即布局（1440×900 下三行：300px / 自适应 / 190px）：

1. **左上（3 列）巨号**：待我动作条数两位数显示（`05`），下接「件事等你处理」与等宽小字「最久 N 天，M 件标在拓扑图上」（窄屏拓扑排在下方，文案不写方位；2026-09-23 收口时改）。
2. **右上（9 列）协作拓扑缩略**（网格纸底）：执行者 → 任务 → 占用路径三列，只画未结束的任务，最多 6 条，超出显示「另 N 条，去协作页」。
3. **左中（6 列）待我动作**：一行一件，圈号＋类型（拍板/合并/在场/验收/排查…取原文 kind）＋标题＋项目。能在拓扑上定位的，圈号是**红色实心**，拓扑上同号红圈标在对应任务或路径旁；定位不到的是黑色空心圈。点击＝打开原文。
4. **右中（6 列）今天的时间线**：按执行者分泳道，黑块＝AI 工作，红块＝你的审批；下接最近三条动态。
5. **左下（5 列）项目明细**：序号、项目、状态方块、备注；前五个，另有「另 N 项」。
6. **右下（7 列）标题栏**：图名「驾驶舱」＋日期＋副本代次；十二格读数（副本新鲜度、每轮同步、同步时限、黑板在跑、agent 程序、人侧发行、会话、执行、可读项目、副本文件、副本大小、未关闭 BUG）。拿不到的值显示 `—` 并在格子 `title` 里写原因，**绝不编造**。

数据来源：`/api/workbench`（项目、待办、计数、新鲜度）、`/api/v1/health`、`GET /api/v1/board/overview`（第 10 节）。
黑板未启用（404）时拓扑格显示「黑板未启用」空态与一句原因，其余格子照常；任何一个接口失败只影响自己的格子。

## 7. 协作

- 子标签（红色 3px 下划线表示当前）：**拓扑 · 任务与回执 · 时间线 · 执行**；右侧等宽汇总「N 条在跑　N 处写域重叠　N 处等你」与两个开关「显示休眠」「只看冲突」。
- 项目选择：拓扑默认「全部项目」（用 overview 接口）；其余三个标签需要具体项目，未选时空态提示去选。
- **拓扑布局**（纯函数，放 `web/src/collab/topologyLayout.ts`，必须有单测）：
  - 三列：执行者 x=0（宽 170）、任务 x=300（宽 240）、占用路径 x=690（宽 300），坐标系 1030 宽，SVG 按宽缩放。
  - 行序：按执行者分组，组内按任务 id 稳定排序；路径行对齐到其任务行，一个任务多条路径时在任务行上下对称展开。
  - 连线全部正交折线：执行者右缘 → 竖线 x=235 → 任务左缘；任务右缘 → 竖线 x=600 → 路径左缘。同一列内不交叉。
  - **写域重叠**：黑板里的 `paths` 是 VaultPath 目录前缀（如 `web`、`docs/m3`），不是 glob。两条未结束任务（含休眠）的路径判定重叠的规则与 control-db 的 `contains` 一致：两条相等，或在 `/` 边界上一条是另一条的前缀，比较前转小写（`web` 包含 `web/src/policyApi.ts`）。命中时路径框红底红框，
    右侧红色括号连起重叠的路径，框下写一句「写域重叠：…」。
  - 休眠（长时间无进度的认领、`needs_*` 以外的停滞）：灰色虚线框，受「显示休眠」开关控制。
  - 选中任务＝黑底反白，右侧 3 列详情：认领者、状态、写域、重叠、回执五字段（状态/改动文件/冲突/验收自评/剩余事项）与动作按钮。
  - 窄屏不画 SVG，改为按执行者分组的列表，重叠用红字标出。
- 「任务与回执」「时间线」「执行」三个标签沿用现有 `BoardBoundary`、`ProjectTimeline`、`ExecutionBoundary`/`LocalDispatch` 的全部行为，只换视觉。

## 8. ⌘K 动作面板

- 取代现有快速切换（`QuickSwitcher`），**保留 BUG-011 的修法**（快捷键监听不挂在状态依赖上）与「不在别的模态上叠开」的规则。
- 样式：640px 宽、2px 黑框、10px 硬偏移阴影、paper 底；输入行 24px/800；结果按「动作 · 跳转 · 文档」分组，选中行黑底反白；匹配字下划红线。
- **只收录接口真实支持的动作**：切换视图/子标签、打开项目来源、搜索并打开文档、刷新快照、跳到某个黑板任务、打开执行候选、打开策略的待处理队列。
  画布上的「合并 PR #35」只是示意，Studiolo 没有 GitHub 接口，**不做**。
- 不可逆或需二次认证的动作（发布/回滚策略、删除文件、落地候选产物）**不在面板里直接执行**：面板显示红色虚线框的「执行预览」，
  回车只把人带到拥有该动作的界面并聚焦那个按钮，确认仍在原界面完成。

## 9. 其余视图

- **项目**：明细表（序号、项目、生命周期、当前版本、进行中、未关闭 BUG、下一里程碑；状态方块）；点项目进入「运作」，运作页用同一网格语法重排结构声明、目录漂移、架构、Cargo、Rust 调用各块。窄屏按项目分组，每个事实带原列名。
- **文档**：左侧搜索/目录树分段，右侧阅读器；阅读器与编辑器是全屏覆盖层（2px 黑框、硬阴影），正文区宽度不超过 72ch。
- **策略**：术语改成人话，原术语用等宽小字跟在后面（如「规则草案 `desired`」「生效中 `effective`」「一次性验证码 `One-time Code`」「撤销接入（实例/类型/全局）」）；扩大项用红色，收窄项与未知一律中性色。
- **会话/因子/恢复码对话框**：沿用行为，换成浮层样式。

## 10. 新接口 `GET /api/v1/board/overview`

跨项目只读汇总，供驾驶舱与协作拓扑一次取全。完整契约在 [`../contracts/board-overview.md`](../contracts/board-overview.md)（随实现落盘）。

- 查询参数：`window_hours`，整数 1–168，默认 24；其他参数一律 400（`deny_unknown_fields`）。查询串提取用 `Result<Query<_>, QueryRejection>`，畸形或越界都走 `crates/local-server/src/history.rs` 的 `bad_query()` 同款 400 中文 Problem（与 T-113 横向修法一致）。
- 未启用黑板 404；许可忙 429；与 `/api/v1/board` 相同的鉴权、许可与策略哈希。
- 响应（字段名 snake_case，前端用封闭解码器，多一个或少一个字段都算无效）：

```json
{
  "generated_at": "2026-09-22T19:31:00Z",
  "window_from": "2026-09-21T19:31:00Z",
  "window_to": "2026-09-22T19:31:00Z",
  "policy_hash": "<hex>",
  "items": [
    {
      "id": "<task id>", "title": "前端全面重设计", "project": "项目/Studiolo",
      "status": "running", "parent": null, "adapter": "claude",
      "holder": "<instance id or null>",
      "holder_lineage": [{ "id": "<instance id>", "agent_type": "claude" }],
      "paths": ["web/**"],
      "attempt": { "phase": "running", "worker": "<worker>", "active": true, "lease_until_ms": 0 },
      "progress": { "tools": 12, "edits": 3, "last_progress_ms": 0 },
      "receipt_status": null
    }
  ],
  "items_truncated": false,
  "events": [
    { "at": "2026-09-22T19:31:00Z", "project": "项目/Studiolo", "task_id": "<task id>",
      "action": "<action>", "phase": "<phase>", "attempt_id": null }
  ],
  "events_truncated": false
}
```

- `items`：所有未结束任务，加上窗口内有事件的已结束任务；最多 500 条，超出置 `items_truncated`。`attempt`/`progress` 可为 `null`，
  `receipt_status` 为 `null` 或 `completed|failed|stopped|abandoned`。
- `events`：窗口内全部项目的任务事件，按 `at` 升序，最多 1000 条（保留最新的），超出置 `events_truncated`。
- 时间戳是 RFC3339 UTC，**可带小数秒**（服务端 `jiff` 输出带小数），解码器必须接受；`generated_at` 等于 `window_to`。`items_truncated=false` 时每条事件的任务都在 `items` 里且项目一致；`events_truncated=false` 时每个已结束项至少有一条事件；`items` 按（project, id）码点序严格递增。假数据也按这三条构造。

## 11. 动效与可达性

- 动效只用于反馈与状态变化：悬停/按下 120ms（背景色、1px 位移），浮层出现 120ms 透明度＋4px 位移，视图切换直接切不做过场。
  拓扑选中、胶囊展开不做弹簧。`prefers-reduced-motion: reduce` 时全部取消位移只留透明度。
- 焦点环：2px `--ink` 实线外偏 2px；在黑底块上用 `--on-ink`。
- 对比度：正文 ≥ 4.5:1（两套主题都测）；红色只配文字或形状，不做唯一信号。
- 图标：巨号、方块、圈号、箭头就是主要图形语言；保留现有 `Icon` 组件的线条图标用于按钮，1.7 描边不变。

## 12. 不变量（改视觉时不许动）

1. 所有接口解码器保持封闭：部分或不一致的响应绝不当成空数据显示。
2. 认证请求在途 ≤ 2（`requestJson` 闸门，BUG-017/ADR-022）；会话每日复核一次（T-104）。
3. BUG-011（⌘K 监听）、BUG-015（位置进 hash）、BUG-016（dev 下策略动作）三处修法保留，对应测试保留并通过。
3a. 前端 ESLint 开启类型化 promise 规则（`no-floating-promises`、`no-misused-promises`、`await-thenable`，承接 T-118），新代码不许违规。
4. 不加载任何远程资源；不写 `<style>` 标签（CSP `style-src 'self'`），样式一律进 CSS 文件，组件自己 `import` 自己的 CSS。
5. 开发用假数据（`STUDIOLO_FIXTURES=1`）只在 `vite serve` 下生效，生产构建里不得出现。
6. 显示的每一个数都来自接口；拿不到就显示 `—` 和原因。
