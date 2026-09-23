import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Documents } from '../components/Documents';
import { fixture, jsonResponse } from './fixtures';
import { treeEntry, treePage } from './treeFixtures';

it('同代切换模式保留查询、项目范围、目录展开与页码，隐藏面板不重复请求', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/search')
      return jsonResponse({
        generation: 7,
        hits: [
          {
            path: '说明.md',
            title: '搜索结果',
            line: 1,
            snippet: '',
            kind: 'path',
          },
        ],
        truncated: false,
      });
    if (url.searchParams.get('path') === '')
      return jsonResponse(
        treePage('', [{ ...treeEntry('资料', 'directory'), child_count: 201 }]),
      );
    return jsonResponse(
      url.searchParams.get('offset') === '200'
        ? treePage('资料', [treeEntry('资料/最后一项.pdf')], {
            offset: 200,
            total: 201,
          })
        : treePage(
            '资料',
            Array.from({ length: 200 }, (_, index) =>
              treeEntry(`资料/附件${index}.pdf`),
            ),
            { total: 201, next_offset: 200 },
          ),
    );
  });
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
  await screen.findByRole('button', { name: '搜索结果，查看原文' });
  expect(
    fetchMock.mock.calls.every(([input]) =>
      String(input).startsWith('/api/search?'),
    ),
  ).toBe(true);
  await user.type(screen.getByRole('searchbox'), '中文路径');
  await screen.findByRole('button', { name: '搜索结果，查看原文' });
  await user.selectOptions(
    screen.getByRole('combobox'),
    fixture.projects[0].id,
  );
  await screen.findByRole('button', { name: '搜索结果，查看原文' });
  await user.click(screen.getByRole('tab', { name: '文件树' }));
  expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
  await user.click(
    await screen.findByRole('button', { name: '展开目录 资料' }),
  );
  await screen.findByRole('button', { name: '附件0.pdf，查看附件信息' });
  await user.click(screen.getByRole('button', { name: '下一页' }));
  await screen.findByRole('button', { name: '最后一项.pdf，查看附件信息' });
  const calls = fetchMock.mock.calls.length;
  await user.click(screen.getByRole('tab', { name: '搜索' }));
  expect(screen.getByRole('searchbox')).toHaveValue('中文路径');
  expect(screen.getByRole('combobox')).toHaveValue(fixture.projects[0].id);
  expect(
    screen.queryByRole('region', { name: '文件树' }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole('tab', { name: '文件树' }));
  expect(screen.getByRole('button', { name: '折叠目录 资料' })).toHaveAttribute(
    'aria-expanded',
    'true',
  );
  expect(
    screen.getByRole('button', { name: '最后一项.pdf，查看附件信息' }),
  ).toBeInTheDocument();
  expect(screen.getByText('201–201 / 201 项')).toBeInTheDocument();
  expect(fetchMock).toHaveBeenCalledTimes(calls);
});

it('不经点标签切到文件树也挂上文件树：直达链接与挂着时改 hash 都渲染出树', async () => {
  const fetchMock = vi.fn<typeof fetch>().mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/search')
      return jsonResponse({ generation: 7, hits: [], truncated: false });
    return jsonResponse(treePage('', [treeEntry('资料', 'directory')]));
  });
  vi.stubGlobal('fetch', fetchMock);
  const documents = (
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={vi.fn()}
      onRefresh={vi.fn()}
    />
  );

  // 直达链接：一挂上就在文件树。
  window.history.replaceState(null, '', '#view=documents&panel=tree');
  const direct = render(documents);
  expect(
    await screen.findByRole('button', { name: '展开目录 资料' }),
  ).toBeInTheDocument();
  expect(screen.getByRole('tab', { name: '文件树' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  direct.unmount();

  // 挂在搜索上时 hash 被改成 panel=tree（⌘K、粘贴链接、前进后退）：文件树不能是空面板。
  window.history.replaceState(null, '', '#view=documents');
  render(documents);
  await screen.findByText('当前快照没有可读取的文档。');
  expect(
    screen.queryByRole('region', { name: '文件树' }),
  ).not.toBeInTheDocument();
  act(() => {
    window.history.replaceState(null, '', '#view=documents&panel=tree');
    window.dispatchEvent(new HashChangeEvent('hashchange'));
  });
  expect(screen.getByRole('tab', { name: '文件树' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  // 标签与面板经 aria-controls / aria-labelledby 连起来：面板的名字就是当前标签。
  expect(screen.getByRole('tabpanel', { name: '文件树' })).toContainElement(
    screen.getByRole('region', { name: '文件树' }),
  );
  expect(
    screen.queryByRole('tabpanel', { name: '搜索' }),
  ).not.toBeInTheDocument();
  expect(
    await screen.findByRole('button', { name: '展开目录 资料' }),
  ).toBeInTheDocument();
  expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
});
