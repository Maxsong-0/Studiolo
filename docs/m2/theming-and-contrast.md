# 主题与对比度（浅色／深色）

> 范围：`web/src/styles/**` 全部 25 份 CSS 与 `web/index.html`。
> 任务：T-062（2026-09-17）。实测方法与数字见第 4、5 节。

## 1. 为什么必须在令牌层做

全仓在本次之前**零处** `prefers-color-scheme`，`tokens.css` 只有一套浅色令牌，
`color-scheme` 写死 `light`。此时任何单页自带 `@media (prefers-color-scheme: dark)`
都是有害的：该页的深色文字会落在仍由浅色令牌绘制的 surface 上，对比度不升反降。

因此本仓的硬规矩是：

- **页面样式文件一律不得出现 `@media (prefers-color-scheme: dark)`**，深浅差异只在
  `tokens.css` 的令牌值里表达；
- **页面样式文件一律不得出现颜色字面量**（`#rgb`、`rgb()`、`hsl()`），
  一律引用令牌。新增颜色先加令牌，再引用。
- 不得用 `!important` 压深色，配色问题一律回到令牌层解决。

守卫方式：`grep -nE '#[0-9a-fA-F]{3,8}\b|rgba?\(|prefers-color-scheme' web/src/styles/*.css`
应当只在 `tokens.css` 命中。T-062 改动后实测：非 `tokens.css` 的 24 份文件残留 **0 处**；
2026-09-18 落地到 main 时（T-075）把其后新增的 `health.css`、`policy.css` 一并纳入，26 份残留仍为 **0 处**。

## 2. 令牌分组

`tokens.css` 的 `:root` 定义浅色，`@media (prefers-color-scheme: dark)` 只重定义同名令牌，
不新增选择器。`color-scheme: light dark`（`index.html` 的 `<meta name="color-scheme">` 同步为
`light dark`，否则首屏在 CSS 就位前会闪白）。

| 组 | 令牌 | 用途 |
|---|---|---|
| 面 | `--body-base` `--body-tint-a/b` `--background` `--surface` `--surface-solid` `--surface-sunken` `--surface-header` `--surface-context` `--hover` `--row-hover` `--nav-hover` `--sidebar-item-hover` `--sidebar` `--sidebar-opaque` `--toolbar` `--toolbar-mobile` `--navbar-mobile` `--button-face` `--well` | 由外到内的背景层级 |
| 遮罩 | `--scrim` `--scrim-strong` `--scrim-soft` | `<dialog>::backdrop` |
| 文字 | `--text` `--text-soft` `--muted` `--muted-contrast` `--muted-soft` `--on-accent` | 正文到装饰文字 |
| 线 | `--border` `--border-contrast` `--border-hover` `--border-strong` `--glass-border` `--highlight-inset` `--highlight-inset-soft` | 描边与内高光 |
| 强调 | `--accent` `--accent-soft` `--accent-gradient-start/end` `--accent-border` `--accent-ring` `--accent-focus-border` `--accent-glow` `--on-accent-well` | 链接、选中、聚焦 |
| 状态 | `--status-ok` `--status-warn` `--status-unknown` `--status-mark` `--error` `--error-soft` `--error-border` | 正常／降级／未知／错误 |
| 高亮 | `--mark` `--mark-text` | 文本比对 |
| 色板 | `--symbol-*`（导航 4 组）`--file-*`（文件类型 6 组） | 前景与底色成对，深色下整体反转 |
| 阴影 | `--shadow` `--shadow-dialog` `--shadow-toolbar` `--shadow-navbar` `--shadow-panel` `--shadow-node` `--shadow-raised` `--shadow-control` `--shadow-button` `--shadow-button-hover` `--shadow-nav-active` | 深色下改为纯黑低透明度 |

`prefers-reduced-transparency` 与 `prefers-contrast: more` 两个块只做令牌重指向
（例如 `--sidebar: var(--sidebar-opaque)`），因此自动对深浅两套值同时生效。

## 3. 浅色零漂移

本次是令牌化而非重新配色：**浅色的每一个令牌值都等于它替换掉的原字面值**，
经脚本逐项核对，非预期差异 0 处。唯二有意改动的是两处本就不达标的浅色值：

| 令牌 | 原值 | 新值 | 浅色对比度 | 说明 |
|---|---|---|---|---|
| `--muted-soft`（面包屑分隔符） | `#a7aeba` | `#828b99` | 2.23:1 → **3.44:1** | 非文本图形，WCAG 1.4.11 要求 3:1 |
| `--status-mark`（生命周期状态点） | `#a4adbc` | `#828b99` | 2.26:1 → **3.44:1** | 同上 |

## 4. 实测对比度

方法：Chromium 真实渲染，用 `getComputedStyle` 取前景色，沿 DOM 向上逐层合成得到有效背景
（含半透明叠加），按 WCAG 2.1 相对亮度公式计算；绝对定位的导航高亮渐变单独按两个端点各算一次。
探针共 53 项 + 导航高亮 6 项，覆盖正文、弱化文字、链接、状态点、错误反馈、比对高亮、
导航与文件类型两套色板、对话框内文本。

四种组合（1536×1024 / 390×844 × 浅色 / 深色）**全部 0 失败，横向溢出 0，控制台零 warn／error**。

各组合最薄的几项：

| 组合 | 最低项 | 实测 | 阈值 |
|---|---|---|---|
| 1536×1024 浅色 | 面包屑分隔符、生命周期点（静态资料） | 3.44:1 | 3:1 |
| 1536×1024 深色 | 面包屑分隔符 | 4.44:1 | 3:1 |
| 1536×1024 深色 | 新鲜度水位说明（`--muted` on `--surface-solid`） | 5.81:1 | 4.5:1 |
| 390×844 浅色 | 面包屑分隔符（移动端工具条底） | 3.20:1 | 3:1 |
| 390×844 深色 | 面包屑分隔符 | 4.50:1 | 3:1 |

导航选中项（白字压在强调渐变上）：

| 主题 | 渐变端 1 | 渐变端 2 |
|---|---|---|
| 浅色 文字 | 5.23:1 | 5.19:1 |
| 深色 文字 | 5.27:1 | 6.50:1 |
| 浅色 符号图标 | 3.97:1 | 3.89:1 |
| 深色 符号图标 | 3.80:1 | 4.45:1 |

深色渐变初版取 `#1f6feb`，白字只有 4.63:1（余量 0.13），已调暗到
`#1c66dc → #1758c4`，余量提到 5.27:1。

### 三态（正常／降级／未知）

前端当前**没有健康页**（`docs/m2/health-and-alerting.md` 在全部分支历史中从未存在，
健康页属 T-048 未开工范围）。现有的三态语义落在项目生命周期列上，实测：

| 态 | 浅色 | 深色 | 阈值 |
|---|---|---|---|
| 活跃（`--accent` 点） | 5.23:1 | 8.08:1 | 3:1 |
| 非活跃（`--status-mark` 点） | 3.44:1 | 4.62:1 | 3:1 |
| 未知（无点，`--status-unknown` 文字） | 4.94:1 | 7.04:1 | 4.5:1 |

为将来的健康页预置了三个语义令牌，取值已验证可直接用：

| 令牌 | 浅色 | 浅色 on `--surface` | 深色 | 深色 on `--surface` |
|---|---|---|---|---|
| `--status-ok` | `#1a7f5a` | 4.97:1 | `#56c294` | 7.51:1 |
| `--status-warn`（降级） | `#8f5e10` | 5.56:1 | `#e8ac4a` | 8.21:1 |
| `--status-unknown` | `#68717e` | 4.94:1 | `#9aa3b0` | 6.49:1 |

**降级态刻意与未知态分色**（暖橙 vs 中性灰），避免「降级」被读成「没数据」。

### 三态信号 `--signal-*`（健康页与策略平面共用，T-075）

健康页（T-048）与策略平面视图（T-069）写于本主题之后，各自带了一套颜色字面值和局部的高对比度分支；
深色下这些字面值落在深色面上只有 2.4–3.6:1。落地时收进令牌层：**浅色值逐字不变（零漂移）**，补深色值，
高对比度沿用本文件的 `-contrast` 配对写法，由 `tokens.css` 的 `prefers-contrast: more` 统一切换。
策略平面的「扩大项」必须是警示红，所以不复用上表的暖橙 `--status-warn`。对比度为计算值（WCAG 相对亮度公式），两个数分别对 `--surface` 与 `--surface-solid`：

| 令牌 | 浅色 | 浅色对比度 | 深色 | 深色对比度 |
|---|---|---|---|---|
| `--signal-ok` | `#1f7a4d` | 5.32:1 ／ 4.96:1 | `#56c294` | 7.51:1 ／ 6.73:1 |
| `--signal-alert` | `#b3452f` | 5.51:1 ／ 5.14:1 | `#f0907a` | 7.06:1 ／ 6.33:1 |
| `--signal-neutral` | `#68717e` | 4.94:1 ／ 4.6:1 | `#9aa3b0` | 6.49:1 ／ 5.81:1 |
| `--signal-ok-contrast` | `#145635` | 8.71:1 ／ 8.12:1 | `#7fdcb0` | 10.05:1 ／ 9.01:1 |
| `--signal-alert-contrast` | `#8a2f1f` | 8.38:1 ／ 7.81:1 | `#ffb3a8` | 9.65:1 ／ 8.65:1 |
| `--signal-neutral-contrast` | `#394452` | 9.89:1 ／ 9.22:1 | `#c9d1dc` | 10.73:1 ／ 9.62:1 |

用途：`--signal-ok` ＝ 健康「正常」、策略「通过」；`--signal-alert` ＝ 健康「降级」、策略「扩大项」；
`--signal-neutral` ＝ 健康「未知」、策略「收窄项」。**「未知」与「收窄」永远是中性色，绝不用绿。**
这两个视图在深色下**没有做真实浏览器探针**（需要带健康证据与策略平面的会话后端），上表是按令牌值算出来的。

## 5. 不按 1.4.11 判定的装饰项

以下是纯装饰描边，层级信息同时由缩进、列表语义或相邻文字表达，不作为唯一状态指示，
按 WCAG 1.4.11「理解内容所必需的图形对象」的限定不纳入 3:1 判定，实测值记录备查：

| 项 | 令牌 | 浅色 | 深色 |
|---|---|---|---|
| 卡片与输入框描边 | `--border` | 1.25:1 | 1.44:1 |
| 运作树连接线 | `--border-strong` | 1.72:1 | 1.73:1 |

如果将来这两者成为唯一的状态区分手段，必须提到 3:1 后再用。

## 6. 新增样式的检查清单

1. 只写 `var(--…)`，不写颜色字面量；缺语义就先在 `tokens.css` 两套值里各加一行。
2. 新令牌落盘后，在浅色与深色下各算一次对比度：正文 ≥4.5:1，大字号与图形 ≥3:1，
   并留出余量（低于 4.8:1 的正文组合建议再调一档）。
3. 跑 `cd web && npm run test && npm run lint && npm run format && npm run build`。
4. 在 1536×1024 与 390×844 两个尺寸、浅色与深色两种系统设置下各看一遍，确认无横向溢出、
   控制台无 warn／error。
