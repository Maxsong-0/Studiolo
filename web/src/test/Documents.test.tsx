import { render, screen } from '@testing-library/react';
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
  await user.click(screen.getByRole('button', { name: '文件树' }));
  expect(screen.queryByRole('searchbox')).not.toBeInTheDocument();
  await user.click(
    await screen.findByRole('button', { name: '展开目录 资料' }),
  );
  await screen.findByRole('button', { name: '附件0.pdf，查看附件信息' });
  await user.click(screen.getByRole('button', { name: '下一页' }));
  await screen.findByRole('button', { name: '最后一项.pdf，查看附件信息' });
  const calls = fetchMock.mock.calls.length;
  await user.click(screen.getByRole('button', { name: '搜索' }));
  expect(screen.getByRole('searchbox')).toHaveValue('中文路径');
  expect(screen.getByRole('combobox')).toHaveValue(fixture.projects[0].id);
  expect(
    screen.queryByRole('region', { name: '文件树' }),
  ).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '文件树' }));
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
