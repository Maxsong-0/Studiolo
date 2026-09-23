import { render, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../components/AppShell';
import { validateWorkbench } from '../validate';
import { fixture } from './fixtures';

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
