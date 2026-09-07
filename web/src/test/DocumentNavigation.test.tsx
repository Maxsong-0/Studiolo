import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { DocumentDialog } from '../components/DocumentDialog';
import { DocumentNavigation } from '../components/DocumentNavigation';
import type { DocumentNavigationData } from '../types';
import { emptyNavigation, jsonResponse } from './fixtures';

const navigation: DocumentNavigationData = {
  outline: [{ title: '二级标题', level: 2, source: { path: 'a.md', line: 3 } }],
  outgoing: [
    {
      label: '前往B',
      raw: '[[b]]',
      source: { path: 'a.md', line: 4 },
      status: 'resolved',
      targets: [{ path: 'b.md', line: 2, title: 'B' }],
    },
    {
      label: '同名笔记',
      raw: '[[same]]',
      source: { path: 'a.md', line: 5 },
      status: 'ambiguous',
      targets: [
        { path: '项目/甲/same.md', line: 1, title: 'same' },
        { path: '项目/乙/same.md', line: 1, title: 'same' },
      ],
    },
    {
      label: '不存在',
      raw: '[[missing]]',
      source: { path: 'a.md', line: 6 },
      status: 'missing',
      targets: [],
    },
    {
      label: '缺失标题',
      raw: '[[b#none]]',
      source: { path: 'a.md', line: 7 },
      status: 'missing_heading',
      targets: [],
    },
    {
      label: '重复标题',
      raw: '[[a#same]]',
      source: { path: 'a.md', line: 8 },
      status: 'ambiguous_heading',
      targets: [
        { path: 'a.md', line: 3, title: 'same' },
        { path: 'a.md', line: 8, title: 'same' },
      ],
    },
  ],
  backlinks: [
    {
      title: '引用本页',
      source: { path: 'c.md', line: 4 },
      context: '<script>alert(1)</script>',
    },
  ],
  truncated: true,
};
const fetchMock = vi.fn<typeof fetch>();

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = new URL(String(input), 'http://localhost').searchParams.get(
      'path',
    );
    return jsonResponse({
      path,
      title: `${path} 原文`,
      content:
        '# 开始\n第二行\n## 二级标题\n第四行\n第五行\n第六行\n第七行\n第八行',
      navigation: path === 'a.md' ? navigation : emptyNavigation,
    });
  });
  vi.stubGlobal('fetch', fetchMock);
});

it('大纲同页定位不重取文档，链接及后退保留原始行号和generation', async () => {
  const user = userEvent.setup();
  render(
    <DocumentDialog
      source={{ path: 'a.md', line: 1 }}
      generation={11}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  await screen.findByRole('heading', { name: 'a.md 原文' });
  expect(screen.getByText('大纲（1）').closest('details')).toHaveAttribute(
    'open',
  );
  await user.click(screen.getByRole('button', { name: /二级标题.*第 3 行/ }));
  expect(
    screen.getByText('a.md · 第 3 行', { selector: '.source-location' }),
  ).toBeInTheDocument();
  expect(fetchMock).toHaveBeenCalledTimes(1);
  await user.click(screen.getByText('本页链接（5）'));
  await user.click(screen.getByRole('button', { name: 'b.md · 第 2 行' }));
  await screen.findByRole('heading', { name: 'b.md 原文' });
  expect(
    screen.getByText('b.md · 第 2 行', { selector: '.source-location' }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '后退' }));
  await screen.findByRole('heading', { name: 'a.md 原文' });
  expect(
    screen.getByText('a.md · 第 3 行', { selector: '.source-location' }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '后退' }));
  expect(
    screen.getByText('a.md · 第 1 行', { selector: '.source-location' }),
  ).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '后退' })).toBeDisabled();
  expect(
    fetchMock.mock.calls.every(
      ([url]) =>
        new URL(String(url), 'http://localhost').searchParams.get(
          'generation',
        ) === '11',
    ),
  ).toBe(true);
});

it('歧义但无可定位候选时明确提示核对原文，不呈现空选择器', () => {
  render(
    <DocumentNavigation
      navigation={{
        ...emptyNavigation,
        outgoing: [{ ...navigation.outgoing[1], targets: [] }],
      }}
      onNavigate={vi.fn()}
    />,
  );
  expect(
    screen.getByText('暂无可定位候选，请核对原文或导航截断提示。'),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('list', { name: /候选目标/ }),
  ).not.toBeInTheDocument();
});

it('歧义需明确选择完整路径，缺失项没有伪造跳转，反链上下文转义', async () => {
  const user = userEvent.setup();
  const { container } = render(
    <DocumentDialog
      source={{ path: 'a.md', line: 1 }}
      generation={11}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  expect(
    await screen.findByText('导航条目较多，当前仅显示部分结果。'),
  ).toBeInTheDocument();
  await user.click(screen.getByText('本页链接（5）'));
  expect(screen.getByText('存在同名文档，请选择完整路径')).toBeInTheDocument();
  expect(screen.getByText('未找到目标文档')).toBeInTheDocument();
  expect(screen.getByText('未找到目标标题')).toBeInTheDocument();
  const missing = screen.getByText('不存在').closest('li')!;
  expect(within(missing).queryByRole('button')).not.toBeInTheDocument();
  expect(
    screen.getByRole('list', { name: '重复标题 的候选目标' }),
  ).toBeInTheDocument();
  await user.click(
    screen.getByRole('button', { name: /项目\/乙\/same.md · 第 1 行/ }),
  );
  await screen.findByRole('heading', { name: '项目/乙/same.md 原文' });
  await user.click(screen.getByRole('button', { name: '后退' }));
  await screen.findByRole('heading', { name: 'a.md 原文' });
  await user.click(screen.getByText('反链（1）'));
  expect(screen.getByText('<script>alert(1)</script>')).toBeInTheDocument();
  expect(container.querySelector('script')).toBeNull();
  await user.click(screen.getByRole('button', { name: /引用本页/ }));
  await screen.findByRole('heading', { name: 'c.md 原文' });
  expect(
    screen.getByText('c.md · 第 4 行', { selector: '.source-location' }),
  ).toBeInTheDocument();
});
