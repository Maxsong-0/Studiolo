import { render, screen, waitFor, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../components/AppShell';
import { validateWorkbench } from '../validate';
import { CockpitHarness } from './cockpit/CockpitHarness';
import { defaultApiResponse, fixture } from './fixtures';

function shell(documentCount: number | null, actionCount: number | null = 0) {
  return (
    <AppShell
      view="cockpit"
      onNavigate={vi.fn()}
      actionCount={actionCount}
      documentCount={documentCount}
    >
      <p>正文</p>
    </AppShell>
  );
}

/** 导航按钮的可访问名只是视图名；计数是旁边的等宽小字，对读屏隐藏。 */
function navCount(view: string) {
  return screen
    .getByRole('button', { name: view })
    .querySelector('.shell-nav-count')?.textContent;
}

it('顶栏文档计数已知时显示数字，不可得时显示占位而不是假装是 0', () => {
  const { rerender } = render(shell(12));
  expect(navCount('文档')).toBe('12');
  rerender(shell(null));
  expect(navCount('文档')).toBe('—');
  rerender(shell(0));
  expect(navCount('文档')).toBe('0');
});

it('驾驶舱旁的待我动作计数在还没有快照时同样显示占位', () => {
  const { rerender } = render(shell(1, 3));
  expect(navCount('驾驶舱')).toBe('3');
  rerender(shell(1, null));
  expect(navCount('驾驶舱')).toBe('—');
  expect(navCount('项目')).toBeUndefined();
});

it('驾驶舱标题栏的副本文件与未关闭 BUG 取自首屏计数：已知给数，不可得显示「—」并列出是哪些项目', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>(async (input) => defaultApiResponse(input)),
  );
  // 黑板汇总由 App 读一次交给驾驶舱（T-162）；单独渲染驾驶舱时由外层代行那一份读取。
  render(
    <CockpitHarness
      workbench={fixture}
      refreshing={false}
      onRefresh={vi.fn()}
      filters={null}
      actions={fixture.inbox}
      projects={fixture.projects}
      diagnostics={0}
      onOpen={vi.fn()}
      onEnterProject={vi.fn()}
      onViewProjects={vi.fn()}
      onShowDiagnostics={vi.fn()}
    />,
  );
  const cell = (label: string) =>
    within(screen.getByRole('region', { name: '图纸标题栏' }))
      .getByText(label)
      .closest('li') as HTMLElement;
  expect(within(cell('副本文件')).getByText('1')).toBeInTheDocument();
  expect(within(cell('可读项目')).getByText('3')).toBeInTheDocument();
  const bugs = cell('未关闭 BUG');
  expect(within(bugs).getByText('—')).toBeInTheDocument();
  expect(bugs).toHaveAttribute(
    'title',
    '1 个项目的未关闭 BUG 数不可得：灵感盒。',
  );
  // 等黑板汇总（测试入口未装配，404）与本机健康读完，免得更新落在测试结束之后。
  await waitFor(() =>
    expect(cell('黑板在跑')).toHaveAttribute('title', '黑板未启用'),
  );
  await waitFor(() =>
    expect(cell('会话')).toHaveAttribute('title', '健康报告未提供'),
  );
});

it('首屏响应带计数即可通过校验，逐篇清单不再是必需字段', () => {
  const payload = {
    generation: fixture.generation,
    source_label: fixture.source_label,
    projects: fixture.projects,
    inbox: fixture.inbox,
    diagnostics: fixture.diagnostics,
    counts: fixture.counts,
  };
  expect(validateWorkbench(payload).counts.documents.value).toBe(1);
  expect(validateWorkbench(payload).counts.active_tasks.value).toBeNull();
});

it('计数缺失、退化成负数或缺少不可得说明的响应一律被拒', () => {
  const base = {
    generation: fixture.generation,
    source_label: fixture.source_label,
    projects: fixture.projects,
    inbox: fixture.inbox,
    diagnostics: fixture.diagnostics,
  };
  const rejected = [
    base,
    { ...base, counts: { ...fixture.counts, documents: undefined } },
    { ...base, counts: { ...fixture.counts, documents: 3 } },
    {
      ...base,
      counts: {
        ...fixture.counts,
        documents: { value: -1, unavailable: [] },
      },
    },
    {
      ...base,
      counts: { ...fixture.counts, documents: { value: null } },
    },
  ];
  for (const payload of rejected) {
    expect(() => validateWorkbench(payload)).toThrowError(
      '工作台数据格式不完整，请刷新或检查服务。',
    );
  }
});

it('旧形状的逐篇清单不能顶替计数通过校验', () => {
  expect(() =>
    validateWorkbench({
      generation: fixture.generation,
      source_label: fixture.source_label,
      projects: fixture.projects,
      inbox: fixture.inbox,
      diagnostics: fixture.diagnostics,
      documents: [{ path: '项目/星舟/00-总览/INDEX.md', title: '星舟总览' }],
    }),
  ).toThrowError('工作台数据格式不完整，请刷新或检查服务。');
});
