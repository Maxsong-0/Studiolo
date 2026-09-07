import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { Documents } from '../components/Documents';
import { fixture, jsonResponse } from './fixtures';

const fetchMock = vi.fn<typeof fetch>();
const onOpen = vi.fn();
const onRefresh = vi.fn();
const hit = {
  path: '项目/星舟/说明.md',
  title: '说明',
  line: 12,
  snippet: '<img src=x onerror=alert(1)> 中文原文',
  kind: 'content',
};
const response = { generation: 7, hits: [hit], truncated: false };

beforeEach(() => {
  fetchMock.mockReset();
  fetchMock.mockImplementation(async () => jsonResponse(response));
  vi.stubGlobal('fetch', fetchMock);
});

it('空查询用路径接口列文档，中文查询与项目范围交给服务端，结果片段只作文本', async () => {
  const user = userEvent.setup();
  const { container } = render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  await screen.findByText('说明');
  const initial = new URL(
    String(fetchMock.mock.calls[0][0]),
    'http://localhost',
  );
  expect(initial.searchParams.get('mode')).toBe('path');
  expect(initial.searchParams.get('q')).toBe('');
  await user.type(screen.getByRole('searchbox'), '中文');
  expect(screen.queryByText('说明')).not.toBeInTheDocument();
  await screen.findByText('说明');
  await user.selectOptions(screen.getByRole('combobox'), 'xingzhou');
  await screen.findByText('说明');
  const latest = new URL(
    String(fetchMock.mock.calls.at(-1)![0]),
    'http://localhost',
  );
  expect(latest.searchParams.get('mode')).toBe('full_text');
  expect(latest.searchParams.get('q')).toBe('中文');
  expect(latest.searchParams.get('project')).toBe('xingzhou');
  expect(latest.searchParams.get('generation')).toBe('7');
  expect(container.querySelector('img')).toBeNull();
  expect(screen.getByText(hit.snippet)).toBeInTheDocument();
  await user.click(screen.getByText('说明'));
  expect(onOpen).toHaveBeenCalledWith({ path: hit.path, line: 12 });
});

it('展示100条截断说明和空态，不伪造下一页', async () => {
  fetchMock.mockResolvedValueOnce(
    jsonResponse({ ...response, truncated: true }),
  );
  const user = userEvent.setup();
  render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  expect(await screen.findByText(/仅显示前 100 条/)).toBeInTheDocument();
  fetchMock.mockResolvedValueOnce(jsonResponse({ ...response, hits: [] }));
  await user.type(screen.getByRole('searchbox'), '没有');
  expect(
    await screen.findByText('没有匹配的文档，请调整搜索词或项目范围。'),
  ).toBeInTheDocument();
});

it('429可以重试，409要求刷新工作台', async () => {
  fetchMock.mockResolvedValueOnce(jsonResponse({ error: 'busy' }, 429));
  const user = userEvent.setup();
  render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  expect(await screen.findByRole('alert')).toHaveTextContent('搜索服务正忙');
  await user.click(screen.getByRole('button', { name: '重试搜索' }));
  await screen.findByText('说明');
  fetchMock.mockResolvedValueOnce(jsonResponse({ error: 'stale' }, 409));
  await user.type(screen.getByRole('searchbox'), '过期');
  expect(await screen.findByRole('alert')).toHaveTextContent('快照已更新');
  await user.click(screen.getByRole('button', { name: '刷新工作台' }));
  expect(onRefresh).toHaveBeenCalledOnce();
});

it('切换快照立即丢弃旧结果，拒绝新请求返回旧generation', async () => {
  const { rerender } = render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  await screen.findByText('说明');
  rerender(
    <Documents
      projects={fixture.projects}
      generation={8}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  expect(screen.queryByText('说明')).not.toBeInTheDocument();
  expect(await screen.findByRole('alert')).toHaveTextContent('不同快照');
});

it('超长搜索词不联网，允许恢复为合法查询', async () => {
  const user = userEvent.setup();
  render(
    <Documents
      projects={fixture.projects}
      generation={7}
      onOpen={onOpen}
      onRefresh={onRefresh}
    />,
  );
  await screen.findByText('说明');
  const calls = fetchMock.mock.calls.length;
  await user.click(screen.getByRole('searchbox'));
  await user.paste('文'.repeat(257));
  expect(screen.getByRole('alert')).toHaveTextContent('最多 256 个字符');
  expect(fetchMock).toHaveBeenCalledTimes(calls);
  await user.clear(screen.getByRole('searchbox'));
  await waitFor(() => expect(screen.getByText('说明')).toBeInTheDocument());
});
