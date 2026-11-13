import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';
import { CapabilityDiff } from '../../policy/CapabilityDiff';
import { compareCapability } from '../../policy/capabilityDiff';

/** 能力差异表（T-198）：模型算出来的每一行怎么写到屏幕上；判定扩大、收窄只照传进来的校验报告。 */

const BASE = {
  ai_type: 'claude',
  read: {
    project_whitelist: ['项目/Studiolo', '项目/旧项目'],
    deny_by_default: true,
    rules_whitelist: ['CLAUDE.md'],
  },
  write: {
    board: false,
    board_projects: [],
    content_patch_only: true,
    content_direct: false,
  },
  policy: { propose_desired: true, activate: false },
  hard_bound_skills: [{ when: '写 React', must_use: 'react-best' }],
};
type Shape = typeof BASE;
const body = (change: (draft: Shape) => void = () => {}) => {
  const draft = structuredClone(BASE);
  change(draft);
  return JSON.stringify(draft);
};

/** 草案：多读一个项目、少读一个项目、打开黑板写入、去掉一条硬绑定。 */
const CHANGED = body((draft) => {
  draft.read.project_whitelist = ['项目/Studiolo', '项目/Semiment'];
  draft.write.board = true;
  draft.hard_bound_skills = [];
});

/** 服务端对上面这份草案的判定。 */
const VERDICTS = {
  expansions: [
    {
      path: 'read.project_whitelist',
      before: null,
      after: '项目/Semiment',
      note: '读取范围变宽',
    },
    {
      path: 'write.board',
      before: 'false',
      after: 'true',
      note: '获得黑板写权限',
    },
    {
      path: 'hard_bound_skills',
      before: '写 React → react-best',
      after: null,
      note: '硬绑定被解除：约束放宽',
    },
  ],
  narrowings: [
    {
      path: 'read.project_whitelist',
      before: '项目/旧项目',
      after: null,
      note: '读取范围收窄',
    },
  ],
};

/** 一行：按行首那一格里字段的名字找（人话名字；不认识的字段是原路径）。 */
function row(name: string) {
  const header = screen
    .getAllByRole('rowheader')
    .find(
      (cell) =>
        cell.querySelector('.policy-capdiff__name')?.textContent === name,
    );
  const line = header?.closest('tr');
  if (!header || !line) throw new Error(`缺少「${name}」这一行`);
  const [before, after] = within(line).getAllByRole('cell');
  return { line, header, before, after };
}

/** 表里各行的字段名，按屏幕上的先后。 */
const names = () =>
  screen
    .getAllByRole('rowheader')
    .map(
      (header) => header.querySelector('.policy-capdiff__name')?.textContent,
    );

/** 「列出／收起不变的 N 项」那个开关。 */
const toggle = () => screen.getByRole('button', { name: /不变的 \d+ 项$/ });

/** 把不变的行也列出来。 */
async function listAll() {
  await userEvent.setup().click(toggle());
}

/** 一格里某一项名单条目所在的那一条。 */
function entry(cell: HTMLElement, text: string) {
  const item = within(cell).getByText(text).closest('li');
  if (!item) throw new Error(`缺少名单项 ${text}`);
  return item;
}

describe('能力差异表', () => {
  it('是一张有名字的表：三列，列头写明两边各比的是哪一版；行首是人话名字加原路径', () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), CHANGED)}
        before="生效中 v2"
        after="这份草案"
        verdicts={VERDICTS}
      />,
    );
    const table = screen.getByRole('table', {
      name: '能力集的每一项：现在和改后',
    });
    expect(
      within(table)
        .getAllByRole('columnheader')
        .map((header) => header.textContent),
    ).toEqual(['能力', '现在生效中 v2', '改后这份草案']);
    const { header, before, after } = row('可读的项目');
    expect(within(header).getByText('read.project_whitelist')).toHaveAttribute(
      'lang',
      'en',
    );
    expect(header).toHaveTextContent('路径前缀名单，名单之外读不到');
    // 窄屏时列头收起来，两格各自带着列名。
    expect(before).toHaveAttribute('data-label', '现在（生效中 v2）');
    expect(after).toHaveAttribute('data-label', '改后（这份草案）');
    expect(screen.getByText('3 项有改动，8 项不变')).toBeInTheDocument();
  });

  it('先只列有改动的行；「列出不变的 N 项」按字段原顺序全部列出，再点收起', async () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), CHANGED)}
        before="生效中 v2"
        after="这份草案"
        verdicts={VERDICTS}
      />,
    );
    const table = screen.getByRole('table', {
      name: '能力集的每一项：现在和改后',
    });
    expect(names()).toEqual(['可读的项目', '可写黑板', '硬绑定的 skill']);
    expect(toggle()).toHaveTextContent('列出不变的 8 项');
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
    expect(toggle()).toHaveAttribute('aria-controls', table.id);

    await listAll();
    expect(names()).toEqual([
      'AI 类型',
      '可读的项目',
      '名单之外一律不可读',
      '可读的规则',
      '可写黑板',
      '可写黑板的项目',
      '改正文只能提交补丁',
      '可整页覆盖正文',
      '可提规则草案',
      '可自行发布',
      '硬绑定的 skill',
    ]);
    // 11 个字段各一行，外加表头一行。
    expect(within(table).getAllByRole('row')).toHaveLength(12);
    expect(toggle()).toHaveTextContent('收起不变的 8 项');
    expect(toggle()).toHaveAttribute('aria-expanded', 'true');
    // 汇总那一句不跟着变。
    expect(screen.getByText('3 项有改动，8 项不变')).toBeInTheDocument();

    await userEvent.setup().click(toggle());
    expect(names()).toEqual(['可读的项目', '可写黑板', '硬绑定的 skill']);
  });

  it('名单逐项标「新增」「去掉」，两边都有的不标；有改动的行写「有改动」，不变的行不写', async () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), CHANGED)}
        before="生效中 v2"
        after="这份草案"
        verdicts={VERDICTS}
      />,
    );
    const projects = row('可读的项目');
    expect(projects.line).toHaveAttribute('data-changed', 'true');
    expect(within(projects.header).getByText('有改动')).toBeInTheDocument();

    const removed = entry(projects.before, '项目/旧项目');
    expect(removed).toHaveAttribute('data-only', 'before');
    expect(within(removed).getByText('去掉')).toBeInTheDocument();
    const added = entry(projects.after, '项目/Semiment');
    expect(added).toHaveAttribute('data-only', 'after');
    expect(within(added).getByText('新增')).toBeInTheDocument();
    for (const cell of [projects.before, projects.after]) {
      const kept = entry(cell, '项目/Studiolo');
      expect(kept).not.toHaveAttribute('data-only');
      expect(kept).toHaveTextContent(/^项目\/Studiolo$/);
    }

    await listAll();
    const rules = row('可读的规则');
    expect(rules.line).not.toHaveAttribute('data-changed');
    expect(within(rules.header).queryByText('有改动')).not.toBeInTheDocument();
  });

  it('「扩大」「收窄」只照校验报告标：标在报告说的那一项上，报告没说的不标', () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), CHANGED)}
        before="生效中 v2"
        after="这份草案"
        verdicts={VERDICTS}
      />,
    );
    const projects = row('可读的项目');
    // 新增的项目：扩大（红色实心方块＝等你确认）。
    const added = within(entry(projects.after, '项目/Semiment'))
      .getByText('扩大')
      .closest('.ui-status');
    expect(added).toHaveAttribute('data-state', 'alert');
    // 去掉的项目：收窄（中性色）。
    const removed = within(entry(projects.before, '项目/旧项目'))
      .getByText('收窄')
      .closest('.ui-status');
    expect(removed).toHaveAttribute('data-state', 'pending');

    // 单值字段：方向只标在「改后」那一格。
    const board = row('可写黑板');
    expect(board.before).toHaveTextContent(/^否$/);
    expect(board.after).toHaveTextContent(/^是扩大$/);

    // 去掉一条硬绑定是放宽约束：服务端判的是扩大，标在「现在」那一列去掉的那一项上。
    const bound = row('硬绑定的 skill');
    const unbound = entry(bound.before, '写 React → react-best');
    expect(within(unbound).getByText('去掉')).toBeInTheDocument();
    expect(within(unbound).getByText('扩大')).toBeInTheDocument();
    expect(bound.after).toHaveTextContent(/^空$/);

    // 整张表里的方向标记就这四处。
    expect(screen.getAllByText('扩大')).toHaveLength(3);
    expect(screen.getAllByText('收窄')).toHaveLength(1);
  });

  it('没给校验报告（正在写、还没提交）：只说新增、去掉、有改动，一个「扩大」「收窄」都不标', () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), CHANGED)}
        before="生效中 v2"
        after="正在写的"
        typing
      />,
    );
    expect(screen.getAllByText('新增')).toHaveLength(1);
    expect(screen.getAllByText('去掉')).toHaveLength(2);
    expect(screen.getAllByText('有改动')).toHaveLength(3);
    expect(screen.queryByText('扩大')).not.toBeInTheDocument();
    expect(screen.queryByText('收窄')).not.toBeInTheDocument();
  });

  it('报告里没有的改动不标方向：新加的项目已被更宽的前缀盖住，服务端不算扩大，这里也不算', () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(
          body(),
          body((draft) => {
            draft.read.project_whitelist.push('项目/Studiolo/子目录');
          }),
        )}
        before="生效中 v2"
        after="这份草案"
        verdicts={{ expansions: [], narrowings: [] }}
      />,
    );
    const added = entry(row('可读的项目').after, '项目/Studiolo/子目录');
    expect(within(added).getByText('新增')).toBeInTheDocument();
    expect(screen.queryByText('扩大')).not.toBeInTheDocument();
    expect(screen.queryByText('收窄')).not.toBeInTheDocument();
  });

  it('单值写成人话：是／否，文字原样；这一边没有就写「—」并给读屏一句「没有」；空名单写「空」', async () => {
    const missing = JSON.parse(body()) as Record<string, unknown>;
    delete (missing.policy as Record<string, unknown>).activate;
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), JSON.stringify(missing))}
        before="生效中 v2"
        after="这份草案"
      />,
    );
    await listAll();
    expect(row('AI 类型').before).toHaveTextContent(/^claude$/);
    expect(row('名单之外一律不可读').after).toHaveTextContent(/^是$/);
    expect(row('可整页覆盖正文').after).toHaveTextContent(/^否$/);
    expect(row('可写黑板的项目').before).toHaveTextContent(/^空$/);

    const activate = row('可自行发布');
    expect(activate.line).toHaveAttribute('data-changed', 'true');
    expect(activate.before).toHaveTextContent(/^否$/);
    expect(within(activate.after).getByText('没有')).toHaveClass('sr-only');
    expect(within(activate.after).getByText('—')).toHaveAttribute(
      'aria-hidden',
      'true',
    );
  });

  it('两边没有一处不同：汇总写「都和现在一样」，不出表；要看就列出来，没有一行标「有改动」', async () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), body())}
        before="生效中 v2"
        after="正在写的"
        typing
      />,
    );
    expect(screen.getByText('11 项都和现在一样')).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(toggle()).toHaveTextContent('列出不变的 11 项');
    // 表还没出来，开关不指向一个不存在的东西。
    expect(toggle()).not.toHaveAttribute('aria-controls');

    await listAll();
    expect(screen.getAllByRole('rowheader')).toHaveLength(11);
    expect(toggle()).toHaveAttribute(
      'aria-controls',
      screen.getByRole('table').id,
    );
    expect(screen.queryByText('有改动')).not.toBeInTheDocument();
    expect(screen.queryByText('新增')).not.toBeInTheDocument();
    expect(screen.queryByText('去掉')).not.toBeInTheDocument();
  });

  it('现在没有生效版本：「现在」一列全是「没有」，「改后」的每一项都是新增', async () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(null, body())}
        before="没有生效版本"
        after="这份草案"
        verdicts={{ expansions: [], narrowings: [] }}
      />,
    );
    const projects = row('可读的项目');
    expect(within(projects.before).getByText('没有')).toBeInTheDocument();
    expect(within(projects.after).getAllByText('新增')).toHaveLength(2);
    expect(projects.before).toHaveAttribute(
      'data-label',
      '现在（没有生效版本）',
    );
    // 空名单对「没有」不算改动：这一行收在「不变的 1 项」里。
    expect(names()).not.toContain('可写黑板的项目');
    await listAll();
    expect(row('可写黑板的项目').line).not.toHaveAttribute('data-changed');
    expect(screen.getByText('10 项有改动，1 项不变')).toBeInTheDocument();
  });

  it('撤销：「改后」一列全是「没有」，「现在」的名单每一项都标去掉', () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), null)}
        before="生效中 v2"
        after="撤销之后"
        verdicts={{
          expansions: [
            { path: '(retire)', before: null, after: null, note: '撤销' },
          ],
          narrowings: [],
        }}
      />,
    );
    const projects = row('可读的项目');
    expect(within(projects.before).getAllByText('去掉')).toHaveLength(2);
    expect(within(projects.after).getByText('没有')).toBeInTheDocument();
    expect(
      screen.getAllByRole('columnheader').map((header) => header.textContent),
    ).toEqual(['能力', '现在生效中 v2', '改后撤销之后']);
    // 报告里那一条撤销不对应表里的任何一项：一个方向都不标。
    expect(screen.queryByText('扩大')).not.toBeInTheDocument();
  });

  it('不认识的字段照样列一行：名字用原路径，写明「不认识的字段，照原样列出」', () => {
    const extended = JSON.parse(body()) as Record<string, unknown>;
    extended.quota = { daily: 5 };
    render(
      <CapabilityDiff
        comparison={compareCapability(body(), JSON.stringify(extended))}
        before="生效中 v2"
        after="这份草案"
      />,
    );
    const quota = row('quota.daily');
    expect(quota.header).toHaveTextContent('不认识的字段，照原样列出');
    expect(quota.line).toHaveAttribute('data-changed', 'true');
    expect(within(quota.before).getByText('没有')).toBeInTheDocument();
    expect(quota.after).toHaveTextContent(/^5$/);
  });

  it('两边都没有内容：不出表，说没有可比的', () => {
    render(
      <CapabilityDiff
        comparison={compareCapability(null, null)}
        before="没有生效版本"
        after="撤销之后"
      />,
    );
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
    expect(
      screen.getByText('两边都没有内容，没有可比的。'),
    ).toBeInTheDocument();
  });
});

describe('读不出结构时不出表，说明原因', () => {
  const cases: {
    name: string;
    before: string | null;
    after: string | null;
    typing: boolean;
    text: string;
  }[] = [
    {
      name: '审草案：草案正文不是 JSON 对象，指给校验报告',
      before: body(),
      after: '不是 JSON',
      typing: false,
      text: '这份草案的正文读不出结构（不是一个 JSON 对象），这张表列不出来；看下面的校验报告。',
    },
    {
      name: '审草案：生效中那一版读不出',
      before: 'v2 正文',
      after: body(),
      typing: false,
      text: '生效中那一版的正文读不出结构，这张表列不出来；看下面的校验报告。',
    },
    {
      name: '正在写：正文还是空的',
      before: body(),
      after: '   ',
      typing: true,
      text: '正文还是空的：写上 JSON 之后，这里会列出它和现在的差别。',
    },
    {
      name: '正在写：JSON 还没写完',
      before: body(),
      after: '{ "ai_type": ',
      typing: true,
      text: '正文还不是一段完整的 JSON：写完整之后，这里会列出差别。',
    },
    {
      name: '正在写：是 JSON 但不是对象',
      before: body(),
      after: '[]',
      typing: true,
      text: '正文不是一个 JSON 对象（要用花括号包起来）：改好之后，这里会列出差别。',
    },
    {
      name: '正在写：生效中那一版读不出，说提交后看校验报告',
      before: 'v2 正文',
      after: body(),
      typing: true,
      text: '生效中那一版的正文读不出结构，这里列不出差别；提交后看校验报告。',
    },
  ];

  it.each(cases)('$name', ({ before, after, typing, text }) => {
    render(
      <CapabilityDiff
        comparison={compareCapability(before, after)}
        before="生效中 v2"
        after={typing ? '正在写的' : '这份草案'}
        typing={typing}
      />,
    );
    expect(screen.getByText(text)).toBeInTheDocument();
    expect(screen.queryByRole('table')).not.toBeInTheDocument();
  });
});
