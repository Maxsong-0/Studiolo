import {
  act,
  render,
  renderHook,
  screen,
  waitFor,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import {
  COCKPIT_READY,
  defaultApiResponse,
  documents,
  emptyNavigation,
  jsonResponse,
} from './fixtures';
import { useReaderSource } from '../useReaderSource';
import {
  defaultViewLocation,
  formatViewLocation,
  parseViewLocation,
} from '../viewLocation';
import type { ViewLocation } from '../viewLocation';

const fetchMock = vi.fn<typeof fetch>();
const path = documents[0].path;

function documentRequests() {
  return fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((url) => url.startsWith('/api/document'));
}

function setHash(hash: string) {
  window.location.hash = hash;
  window.dispatchEvent(new HashChangeEvent('hashchange'));
}

beforeEach(() => {
  window.location.hash = '';
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) =>
    String(input).startsWith('/api/document')
      ? jsonResponse({
          path,
          title: '星舟总览',
          content: '# 星舟',
          navigation: emptyNavigation,
        })
      : defaultApiResponse(input),
  );
  vi.stubGlobal('fetch', fetchMock);
});

describe('文档视图的单文件深链', () => {
  it('在文档视图打开一篇，地址里记下它的路径；关掉就清掉', async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole('heading', COCKPIT_READY);
    await user.click(screen.getByRole('button', { name: '文档' }));
    await user.click(
      await screen.findByRole('button', { name: '星舟总览，查看原文' }),
    );
    expect(await screen.findByText('# 星舟')).toBeInTheDocument();
    expect(parseViewLocation(window.location.hash)).toMatchObject({
      view: 'documents',
      item: path,
    });

    await user.click(screen.getByRole('button', { name: '关闭原文' }));
    expect(parseViewLocation(window.location.hash).item).toBeNull();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('带着路径的链接打开就是那一篇，不用先点一遍', async () => {
    window.location.hash = formatViewLocation({
      view: 'documents',
      project: null,
      panel: null,
      item: path,
    });
    render(<App />);
    expect(await screen.findByText('# 星舟')).toBeInTheDocument();
    expect(
      screen.getByRole('heading', { level: 1, name: '文档' }),
    ).toBeInTheDocument();
    const [request] = documentRequests();
    expect(
      new URL(request, 'https://local.test').searchParams.get('path'),
    ).toBe(path);
  });

  it('地址里的路径没了，阅读器跟着关；换成另一篇就读另一篇', async () => {
    window.location.hash = formatViewLocation({
      view: 'documents',
      project: null,
      panel: null,
      item: path,
    });
    render(<App />);
    expect(await screen.findByRole('dialog')).toBeInTheDocument();

    act(() => setHash('view=documents'));
    await waitFor(() =>
      expect(screen.queryByRole('dialog')).not.toBeInTheDocument(),
    );

    act(() =>
      setHash(
        formatViewLocation({
          view: 'documents',
          project: null,
          panel: null,
          item: '项目/别处/另一篇.md',
        }),
      ),
    );
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    const last = documentRequests().at(-1)!;
    expect(new URL(last, 'https://local.test').searchParams.get('path')).toBe(
      '项目/别处/另一篇.md',
    );
  });

  it('在别的视图打开来源不改地址：那里的条目格另有含义', async () => {
    const user = userEvent.setup();
    render(<App />);
    await screen.findByRole('heading', COCKPIT_READY);
    await user.click(screen.getByRole('button', { name: '项目' }));
    await user.click(
      await screen.findByRole('button', {
        name: '星舟 当前版本：M1，查看来源',
      }),
    );
    expect(await screen.findByRole('dialog')).toBeInTheDocument();
    expect(parseViewLocation(window.location.hash)).toMatchObject({
      view: 'projects',
      item: null,
    });
    await user.click(screen.getByRole('button', { name: '关闭原文' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('放不进地址的路径（超长）照常打开，只是不记位置；换了视图不会在别处弹出来', () => {
    const long = `项目/${'长'.repeat(600)}.md`;
    const select = vi.fn();
    const documentsView: ViewLocation = {
      ...defaultViewLocation,
      view: 'documents',
    };
    const { result, rerender } = renderHook(
      ({ location }) => useReaderSource(location, select),
      { initialProps: { location: documentsView } },
    );
    act(() => result.current.open({ path: long, line: 7 }));
    expect(select).toHaveBeenLastCalledWith({ item: null });
    expect(result.current.source).toEqual({ path: long, line: 7 });

    // 地址里后来有了路径（后退、手改）：以地址为准，不停在没记进位置的那一篇上。
    rerender({ location: { ...documentsView, item: path } });
    expect(result.current.source).toEqual({ path, line: 1 });
    rerender({ location: documentsView });
    expect(result.current.source).toEqual({ path: long, line: 7 });

    rerender({ location: { ...defaultViewLocation, view: 'projects' } });
    expect(result.current.source).toBeNull();
  });

  it('同一篇保留打开时的行号；地址换了路径就从头读那一篇', () => {
    const select = vi.fn();
    const at = (item: string | null): ViewLocation => ({
      ...defaultViewLocation,
      view: 'documents',
      item,
    });
    const { result, rerender } = renderHook(
      ({ location }) => useReaderSource(location, select),
      { initialProps: { location: at(null) } },
    );
    act(() => result.current.open({ path, line: 12 }));
    expect(select).toHaveBeenLastCalledWith({ item: path });
    rerender({ location: at(path) });
    expect(result.current.source).toEqual({ path, line: 12 });
    rerender({ location: at('项目/别处/另一篇.md') });
    expect(result.current.source).toEqual({
      path: '项目/别处/另一篇.md',
      line: 1,
    });
    act(() => result.current.close());
    expect(select).toHaveBeenLastCalledWith({ item: null });
  });
});
