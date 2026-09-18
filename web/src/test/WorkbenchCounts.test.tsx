import { render } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../components/AppShell';
import { validateWorkbench } from '../validate';
import { fixture } from './fixtures';

function shell(documentCount: number | null) {
  return (
    <AppShell
      view="overview"
      onNavigate={vi.fn()}
      sourceLabel="测试库"
      projects={[]}
      onOpen={vi.fn()}
      actionCount={0}
      documentCount={documentCount}
      toolbar={null}
    >
      <p>正文</p>
    </AppShell>
  );
}

it('侧栏文档计数已知时显示数字，不可得时显示占位而不是假装是 0', () => {
  const { rerender, container } = render(shell(12));
  const count = () =>
    container.querySelector('.navigation-item:last-of-type .nav-count')
      ?.textContent;
  expect(count()).toBe('12');
  rerender(shell(null));
  expect(count()).toBe('—');
  rerender(shell(0));
  expect(count()).toBe('0');
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
