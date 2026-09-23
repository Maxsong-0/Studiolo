/**
 * 假 Vault：几篇 Markdown 笔记与两个附件。目录树、搜索、原文、编辑器、页面历史、
 * 工作台首屏（项目字段与待我动作的来源行号）都从这里派生，彼此一致。
 *
 * 日志文件名随「今天」变化（按 UTC 日期），项目时间回放才能在最近 24 小时里找到日志。
 */
import { DAY, utcDate } from './clock.ts';

export interface VaultNote {
  path: string;
  content: string;
}

export interface VaultAttachment {
  path: string;
  bytes: number;
}

/** 八个项目：顺序即后端 `项目/` 下的字典序无关，界面自己排序。 */
export const PROJECT_NAMES = [
  'Studiolo',
  'Semiment',
  'Brief 简报站',
  'AI 网关门户',
  'DocReader',
  'NAS 网关',
  'Tillerwright',
  'AI Quota Flow',
] as const;

export const root = (name: string) => `项目/${name}`;

function index(
  name: string,
  fields: { lifecycle: string; version?: string; state: string },
  lines: string[],
): VaultNote {
  return {
    path: `${root(name)}/00-总览/INDEX.md`,
    content: [
      `# ${name} 总览`,
      '',
      '## 基本信息',
      '',
      `- 生命周期：${fields.lifecycle}`,
      ...(fields.version ? [`- 当前版本：${fields.version}`] : []),
      `- 状态：${fields.state}`,
      '',
      ...lines,
    ].join('\n'),
  };
}

function tasks(name: string, rows: [string, string][]): VaultNote {
  return {
    path: `${root(name)}/04-协调/TASKS.md`,
    content: [
      `# ${name} 协调`,
      '',
      '## ① 进行中',
      '',
      '| 任务 | 状态 |',
      '|---|---|',
      ...rows.map(([task, status]) => `| ${task} | ${status} |`),
      '',
    ].join('\n'),
  };
}

function roadmap(name: string, rows: [string, string][]): VaultNote {
  return {
    path: `${root(name)}/05-版本/roadmap.md`,
    content: [
      `# ${name} 路线图`,
      '',
      '| 里程碑 | 状态 |',
      '|---|---|',
      ...rows.map(([milestone, status]) => `| ${milestone} | ${status} |`),
      '',
    ].join('\n'),
  };
}

function bug(name: string, file: string, title: string, body: string[]) {
  return {
    path: `${root(name)}/06-BUG/${file}`,
    content: ['---', 'status: fixed', '---', `# ${title}`, '', ...body].join(
      '\n',
    ),
  };
}

/** 构造整库笔记。`now` 只决定两篇日志的日期。 */
export function buildVault(now: number): VaultNote[] {
  const today = utcDate(now);
  const yesterday = utcDate(now - DAY);
  const studiolo = root('Studiolo');
  return [
    {
      path: '_规则/通用工作规则.md',
      content: [
        '# 通用工作规则',
        '',
        '全局 CLAUDE.md 的完整母本。权威顺序：用户当场指令 ＞ 项目 CLAUDE.md ＞ 全局规则 ＞ 项目档案。',
        '',
        '## 核心准则',
        '',
        '1. 永远说中文；代码标识符保持英文。',
        '2. 最严谨修复，禁止绕过式补丁；修前 grep 同类实例一并修全。',
        '3. 写码前完全理解需求，动手前用两到五行说明方案。',
        '',
        '## 并发协作',
        '',
        '开工先读 INDEX 与 TASKS，查重后细粒度认领；共享热点文件只用增量编辑。',
        '',
      ].join('\n'),
    },
    {
      path: '_模板/README.md',
      content: [
        '# 项目骨架模板',
        '',
        '新项目从 `_模板/项目骨架/` 复制到 `项目/<项目名>/`，骨架编号与名称禁止擅改：',
        '',
        '- 00-总览、01-框架、02-日志、03-记忆、04-协调、05-版本、06-BUG',
        '',
      ].join('\n'),
    },
    index(
      'Studiolo',
      {
        lifecycle: '活跃',
        version: 'M6（开发分支）',
        state: '🟡 T-041～T-046 已合入 main；T-130 前端全面重设计进行中',
      },
      [
        '> 个人知识库与多 Agent 协作工作台：本机只读工作台、协作黑板、策略平面与副本供给。',
        '',
        '## 待我动作',
        '',
        '- [ ] 副本同步每轮全量重推：拍板选定增量方案（T-108 后续）',
        '- [ ] 合并 PR #35（T-106、T-108 安装文档与副本扩容），CI 全绿',
        '- [ ] 在场完成 T-093：恢复真人 L3 与 RTO/RPO 演练',
        '- [ ] 公网实测验收：策略页刷新不报错',
        '',
        '## 当前重点',
        '',
        '- T-130 前端全面重设计：[[t130-design-system|瑞士网格＋工程图纸设计规范]] 已定稿，15 个代理分三轮实施。',
        '- T-109 全项目未完成项盘点，结果回写各项目 04-协调/TASKS.md。',
        '- 副本同步每轮 85 秒，逼近 120 秒时限，方案见 [[ADR-022-认证许可排队与会话每日复核]]。',
        '- 看板与写域见 [[TASKS]]；里程碑见 [[项目/Studiolo/05-版本/roadmap|路线图]]。',
        '',
        '## 最近日志',
        '',
        `- [[${today}]]`,
        `- [[${yesterday}]]`,
        '',
        '## 档案沿革',
        '',
        '- 2026-08-02 迁入 项目/Studiolo/。',
        '',
      ],
    ),
    {
      path: `${studiolo}/01-框架/目录结构.md`,
      content: [
        '# Studiolo 目录结构',
        '',
        '## 代码仓库',
        '',
        '```text',
        'studiolo/',
        '├── crates/ # Rust 工作区成员',
        '│   ├── local-server/ # 本机工作台 HTTP 入口：会话门禁、黑板、策略、健康',
        '│   ├── vault-broker/ # Vault 读写唯一通道：revision、快照、候选产物',
        '│   ├── task-engine/ # 黑板任务状态机、认领、回执与审计',
        '│   ├── policy/ # 策略平面：desired、校验、发布、三级吊销',
        '│   ├── supply/ # 副本供给：打包、校验、代次与新鲜度',
        '│   └── agent-server/ # 接入方服务：MCP 路牌、只读供给、提案',
        '├── web/ # 工作台前端（React 19 + Vite）',
        '├── deploy/ # 部署脚本与 systemd 单元',
        '└── scripts/ # 副本同步与本机测试脚本',
        '```',
        '',
      ].join('\n'),
    },
    {
      path: `${studiolo}/01-框架/项目运作.md`,
      content: [
        '# Studiolo 项目运作',
        '',
        '## 节点',
        '',
        '| ID | 名称 | 类型 | 目录 | 职责 |',
        '|---|---|---|---|---|',
        '| web | 工作台前端 | 接口 | web | 驾驶舱、协作、文档与策略界面 |',
        '| local | 本机服务 | 模块 | crates/local-server | 会话门禁后的全部 HTTP 接口 |',
        '| broker | Vault Broker | 模块 | crates/vault-broker | 唯一的 Vault 读写通道 |',
        '| engine | 任务引擎 | 模块 | crates/task-engine | 黑板任务、认领与回执 |',
        '| policy | 策略平面 | 模块 | crates/policy | desired 校验与 L3 发布 |',
        '| supply | 副本供给 | 模块 | crates/supply | 副本打包、校验与代次 |',
        '| agent | 接入方服务 | 接口 | crates/agent-server | 接入方的只读供给与提案入口 |',
        '| vault | Vault 原文 | 数据 | - | Obsidian Vault 里的 Markdown 原文 |',
        '| owner | 主人 | 外部 | - | 拍板、审阅与二次认证 |',
        '',
        '## 关系',
        '',
        '| 起点 | 终点 | 类型 | 说明 |',
        '|---|---|---|---|',
        '| web | local | 依赖 | 调用会话门禁后的接口 |',
        '| local | broker | 依赖 | 读原文、写 revision |',
        '| local | engine | 依赖 | 黑板读写与执行控制 |',
        '| engine | broker | 依赖 | 冻结输入快照、提交候选 |',
        '| local | policy | 依赖 | 读 effective、发布与吊销 |',
        '| broker | vault | 数据流 | 读写 Markdown 原文 |',
        '| supply | agent | 数据流 | 按代次供给只读副本 |',
        '| owner | web | 数据流 | 拍板、审阅、One-time Code |',
        '',
        '## 任务路径',
        '',
        '| 流程ID | 流程名称 | 步骤 | 节点ID | 说明 |',
        '|---|---|---|---|---|',
        '| landing | 候选落地 | 1 | engine | 执行体提交候选（candidate.patch ＋ patch.json） |',
        '| landing | 候选落地 | 2 | web | 主人审阅候选与验收自评 |',
        '| landing | 候选落地 | 3 | local | 二次认证后落地到目标仓库 |',
        '| landing | 候选落地 | 4 | engine | 写入收尾回执，任务结束 |',
        '| publish | 策略发布 | 1 | agent | 接入方提交 desired 提案 |',
        '| publish | 策略发布 | 2 | policy | 校验并标出权限扩大项 |',
        '| publish | 策略发布 | 3 | web | 主人逐条确认扩大项 |',
        '| publish | 策略发布 | 4 | policy | One-time Code 通过后发布为新版本 |',
        '',
      ].join('\n'),
    },
    {
      path: `${studiolo}/01-框架/技术栈.md`,
      content: [
        '# 技术栈',
        '',
        '| 层 | 选型 | 版本 |',
        '|---|---|---|',
        '| 后端 | Rust（edition 2024） | 1.97 |',
        '| HTTP | axum | 0.8.9 |',
        '| 前端 | React | 19.2.8 |',
        '| 构建 | Vite | 8.2.2 |',
        '| 测试 | Vitest | 5.0.0 |',
        '',
        '依赖版本在工作区 Cargo.toml 统一写死（ADR-001），精确解析以 Cargo.lock 为准。',
        '',
      ].join('\n'),
    },
    {
      path: `${studiolo}/02-日志/${today}.md`,
      content: [
        `# ${today} 日志`,
        '',
        '## 做了什么',
        '',
        '- T-130 设计规范定稿：A2 管骨架，A4 管表达；驾驶舱、协作页、⌘K 面板三张主稿。',
        '- T-108、T-106 候选提交，等 PR #35 合并；CI 在 githubci 与 github-2 上全绿。',
        '- T-092 审阅通过，写域 web/src/policyApi.ts 尚未释放，与 T-130 的 web 重叠。',
        '',
        '## 踩坑',
        '',
        '- 副本同步每轮 85 秒，离 120 秒时限只剩 35 秒余量；全量重推不可持续。',
        '',
        '## 下一步',
        '',
        '- 拍板增量同步方案；T-093 需要在场完成真人 L3 演练。',
        '- 见 [[ADR-022-认证许可排队与会话每日复核]]。',
        '',
      ].join('\n'),
    },
    {
      path: `${studiolo}/02-日志/${yesterday}.md`,
      content: [
        `# ${yesterday} 日志`,
        '',
        '## 做了什么',
        '',
        '- T-105 认证许可排队上线：在途最多两个，第三个请求排队而不是 503。',
        '- 会话每日复核一次（T-104），切回页面不再清空内容。',
        '',
        '## 下一步',
        '',
        '- T-109 全项目未完成项盘点。',
        '',
      ].join('\n'),
    },
    {
      path: `${studiolo}/02-日志/草稿-未归档.md`,
      content: [
        '# 未归档草稿',
        '',
        '这篇日志没有日期前缀，时间回放不会猜测它属于哪一天。',
        '',
      ].join('\n'),
    },
    {
      path: `${studiolo}/03-记忆/ADR-022-认证许可排队与会话每日复核.md`,
      content: [
        '# ADR-022 认证许可排队与会话每日复核',
        '',
        '- 状态：已采纳',
        '- 日期：2026-09-21',
        '',
        '## 背景',
        '',
        '服务端认证入口同一时刻只校验两个会话，第三个同时到达的请求直接 503。',
        '',
        '## 决定',
        '',
        '1. 前端所有认证请求经同一个闸门：在途最多两个，其余按到达顺序排队。',
        '2. 切回页面时会话一天内确认过就不再核对；任何 401 立即清空内容。',
        '',
        '## 后果',
        '',
        '- 策略页三连读取改为顺序执行，避免自触发 503。',
        '- 副本同步耗时逼近时限的问题另立 [[TASKS|T-108 后续]] 拍板。',
        '',
      ].join('\n'),
    },
    tasks('Studiolo', [
      ['T-130 前端全面重设计', '进行中'],
      ['T-109 全项目未完成项盘点', '进行中'],
      ['T-108 副本扩容纳入 Semiment-Docs', '候选待合并'],
      ['T-106 安装文档补运行包冻结', '候选待合并'],
      ['T-092 人侧 M4 L3 部署', '已验收，写域未释放'],
      ['T-093 M4 异机灾备与恢复演练', '等在场演练'],
      ['T-046 M2/M3 整体收尾', '认领后停滞'],
    ]),
    roadmap('Studiolo', [
      ['M5 协作黑板', '已完成'],
      ['M6 真实执行体', '已完成'],
      ['T-130 前端全面重设计', '进行中'],
      ['M7 跨项目驾驶舱', '规划中'],
    ]),
    bug(
      'Studiolo',
      'BUG-015-切走再回来回落首页.md',
      'BUG-015 切走再回来回落首页',
      [
        '## 现象',
        '',
        '切到别的标签页再回来，工作台回到首页，刚才所在的视图丢失。',
        '',
        '## 根因',
        '',
        '会话重新核对会整页重挂，而位置只存在组件状态里。',
        '',
        '## 修复',
        '',
        '位置写进 hash，重挂后按 hash 回到原处；新增视图同样适用。',
        '',
      ],
    ),
    index(
      'Semiment',
      {
        lifecycle: '已交付维护',
        version: 'v2.4.0',
        state: '🔴 生产站当前对外不可达（2026-09-17 实测）',
      },
      [
        '## 待我动作',
        '',
        '- [ ] 排查生产站 admin / api / docs 对外不可达（2026-09-17 实测超时）',
        '',
      ],
    ),
    tasks('Semiment', [
      ['恢复生产站对外访问', '进行中'],
      ['规格书替换对照表复核', '进行中'],
    ]),
    roadmap('Semiment', [
      ['v2.4.0 规格书替换', '已完成'],
      ['恢复生产站对外访问', '进行中'],
    ]),
    index(
      'Brief 简报站',
      {
        lifecycle: '已交付维护',
        version: 'v0.3.0',
        state: '🔵 已上线（2026-09-17 部署，公网验收通过）',
      },
      ['17 个新闻分类，地铁线路图自适应。', ''],
    ),
    tasks('Brief 简报站', [['v0.3.1 分类订阅', '进行中']]),
    roadmap('Brief 简报站', [
      ['v0.3.0 公网上线', '已完成'],
      ['v0.3.1 分类订阅', '规划中'],
    ]),
    index(
      'AI 网关门户',
      {
        lifecycle: '暂停',
        version: 'S10 智能路由',
        state: '🟢 S10 智能路由已本机收官（2026-09-04）',
      },
      ['model=auto 按任务类型选路，本机验收通过。', ''],
    ),
    tasks('AI 网关门户', [['S10 智能路由', '已完成']]),
    roadmap('AI 网关门户', [
      ['S10 智能路由', '已完成'],
      ['S11 公网部署评估', '规划中'],
    ]),
    index(
      'DocReader',
      { lifecycle: '活跃', version: 'v0.4.0', state: '🟢 开发中' },
      ['v0.4.0 已合并 master；目录导航重排进行中。', ''],
    ),
    tasks('DocReader', [
      ['目录导航重排', '进行中'],
      ['批注导出', '进行中'],
      ['阅读进度同步', '规划中'],
    ]),
    roadmap('DocReader', [
      ['v0.4.0 阅读器重写', '已完成'],
      ['v0.5.0 目录导航重排', '进行中'],
    ]),
    bug('DocReader', 'BUG-002-长文档滚动跳动.md', 'BUG-002 长文档滚动跳动', [
      '图片懒加载改变高度导致滚动位置跳动；改为预留尺寸。',
      '',
    ]),
    index(
      'NAS 网关',
      {
        lifecycle: '已交付维护',
        version: 'v0.1.0',
        state: '🔵 已上线（白名单登录待用户实测）',
      },
      ['nas.maxsong.me → Access 邮箱验证 → 内网 NAS。', ''],
    ),
    tasks('NAS 网关', [['白名单登录实测', '进行中']]),
    roadmap('NAS 网关', [
      ['v0.1.0 上线', '已完成'],
      ['v0.2.0 白名单登录实测', '规划中'],
    ]),
    index(
      'Tillerwright',
      { lifecycle: '可用', version: '0.3.0-dev', state: '🟢 可用' },
      ['已装 /Applications/Tillerwright Dev.app。', ''],
    ),
    tasks('Tillerwright', [['菜单栏快捷操作', '进行中']]),
    bug(
      'Tillerwright',
      'BUG-001-上游源码缺失导致编译失败.md',
      'BUG-001 上游源码缺失导致编译失败',
      ['上游仓库三处源码缺失；已补齐并固定到提交。', ''],
    ),
    index(
      'AI Quota Flow',
      {
        lifecycle: '已交付维护',
        version: 'v1.3.0',
        state: '🔵 已上线（自用）· 持续迭代',
      },
      ['186 个测试全绿，已重装。', ''],
    ),
    tasks('AI Quota Flow', [['T-050 审查驳回项全修', '已完成']]),
    roadmap('AI Quota Flow', [
      ['v1.3.0 驳回项全修', '已完成'],
      ['v1.4.0 额度预测', '规划中'],
    ]),
    bug(
      'AI Quota Flow',
      'BUG-004-额度重置时区错误.md',
      'BUG-004 额度重置时区错误',
      ['重置时刻按 UTC 计算，改为按账户时区。', ''],
    ),
  ];
}

export const ATTACHMENTS: VaultAttachment[] = [
  { path: '项目/Studiolo/01-框架/视觉方向第三轮.png', bytes: 482_133 },
  { path: '项目/Brief 简报站/05-版本/v0.3.0 发布截图.png', bytes: 219_804 },
];

/** 「最近删除」里的一篇：删除前的原文，供恢复时原样放回。 */
export const DELETED_NOTE: VaultNote = {
  path: '项目/Studiolo/04-协调/交接-T-101.md',
  content: [
    '# 交接 T-101',
    '',
    '路牌清单化与全项目白名单已上线，本交接归档后删除。',
    '',
  ].join('\n'),
};

/** 标题：首个一级标题，否则文件名去掉 `.md`。 */
export function noteTitle(note: VaultNote): string {
  const heading = note.content
    .split('\n')
    .find((line) => line.startsWith('# '));
  return heading
    ? heading.slice(2).trim()
    : note.path.split('/').at(-1)!.replace(/\.md$/i, '');
}

/** 1 起算：第一行以 `prefix` 开头（去掉列表与复选框前缀后）的行号。 */
export function lineOf(content: string, prefix: string): number {
  const index = content.split('\n').findIndex((line) =>
    line
      .replace(/^- (\[ \] )?/, '')
      .replace(/^\| /, '')
      .startsWith(prefix),
  );
  if (index < 0) throw new Error(`假数据笔记缺少「${prefix}」这一行`);
  return index + 1;
}
