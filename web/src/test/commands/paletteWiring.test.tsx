import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../../App';
import { DraftContext } from '../../draftContext';
import { DraftStore } from '../../draftStore';
import { FileOperationContext } from '../../fileOperationContext';
import { FileOperationStore } from '../../fileOperationStore';
import { parseViewLocation } from '../../viewLocation';
import { canonical, ownerScope } from '../canonicalFixtures';
import {
  defaultApiResponse,
  documents,
  emptyNavigation,
  fixture,
  jsonResponse,
} from '../fixtures';
import { manyProjects, overviewResponse, taskId } from './paletteFixtures';

type Route = (input: string) => Response | undefined;

const fetchMock = vi.fn<typeof fetch>();
let route: Route;

beforeEach(() => {
  route = () => undefined;
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = String(input);
    const routed = route(path);
    if (routed) return routed;
    if (path.startsWith('/api/document'))
      return jsonResponse({
        path: documents[0].path,
        title: '星舟总览',
        content: '只读文档',
        navigation: emptyNavigation,
      });
    return defaultApiResponse(input);
  });
  vi.stubGlobal('fetch', fetchMock);
});

async function openPalette(ui: ReactNode = <App />) {
  const user = userEvent.setup();
  render(ui);
  // 等驾驶舱的项目表出来即工作台已读到，入口才可用。
  await screen.findByRole('table');
  await user.click(screen.getByRole('button', { name: /^动作面板/ }));
  const dialog = screen.getByRole('dialog', { name: '动作面板' });
  const input = within(dialog).getByRole('combobox', {
    name: '跳转、搜索或执行动作',
  });
  const list = within(dialog).getByRole('listbox');
  return { user, dialog, input, list };
}

function location() {
  return parseViewLocation(window.location.hash);
}

describe('动作面板接进外壳', () => {
  it('跳转：协作 › 任务与回执写进位置并关面板，焦点回到入口', async () => {
    const { user, input, list } = await openPalette();
    await user.type(input, '任务与回执');
    expect(
      within(list).getByRole('option', { name: '协作 › 任务与回执' }),
    ).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{Enter}');
    expect(location()).toEqual({
      view: 'collab',
      project: null,
      panel: 'board',
      item: null,
    });
    expect(
      screen.queryByRole('dialog', { name: '动作面板' }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole('button', { name: /^动作面板/ })).toHaveFocus();
    expect(screen.getByRole('button', { name: '协作' })).toHaveAttribute(
      'aria-current',
      'page',
    );
  });

  it('项目：打开项目进运作；打开项目来源读原文', async () => {
    const first = await openPalette();
    await first.user.type(first.input, '星舟');
    await first.user.click(
      within(first.list).getByRole('option', { name: '项目 › 星舟' }),
    );
    expect(location()).toEqual({
      view: 'projects',
      project: '项目/星舟',
      panel: 'operations',
      item: null,
    });
    await first.user.click(screen.getByRole('button', { name: /^动作面板/ }));
    const dialog = screen.getByRole('dialog', { name: '动作面板' });
    await first.user.type(
      within(dialog).getByRole('combobox', { name: '跳转、搜索或执行动作' }),
      '来源 星舟',
    );
    await first.user.click(
      within(dialog).getByRole('option', { name: '项目来源 › 星舟' }),
    );
    expect(await screen.findByText('只读文档')).toBeInTheDocument();
    const read = fetchMock.mock.calls.find(([input]) =>
      String(input).startsWith('/api/document'),
    )!;
    const params = new URL(String(read[0]), 'http://localhost').searchParams;
    expect(params.get('path')).toBe(documents[0].path);
    expect(params.get('generation')).toBe('7');
  });

  it('项目多于 8 个：组内给出「全部 N 个项目」，跳到项目视图', async () => {
    const projects = manyProjects(10);
    route = (path) =>
      path.startsWith('/api/workbench')
        ? jsonResponse({ ...fixture, projects })
        : undefined;
    const { user, list } = await openPalette();
    const group = within(list).getByRole('group', { name: '跳转' });
    expect(
      within(group)
        .getAllByRole('option')
        .filter((option) =>
          option.getAttribute('aria-label')?.startsWith('项目 › '),
        ),
    ).toHaveLength(8);
    const all = within(group).getByRole('option', { name: '全部 10 个项目' });
    await user.click(all);
    expect(location().view).toBe('projects');
  });

  it('打开状态详情：展开顶栏胶囊并把焦点交给浮层', async () => {
    const { user, input } = await openPalette();
    await user.type(input, '状态详情');
    await user.keyboard('{Enter}');
    const capsule = screen.getByRole('button', { name: /^工作台状态/ });
    expect(capsule).toHaveAttribute('aria-expanded', 'true');
    const popover = document.getElementById(
      capsule.getAttribute('aria-controls')!,
    )!;
    await waitFor(() => expect(popover).toHaveFocus());
  });

  it('刷新快照：重读工作台并关面板', async () => {
    const { user, input } = await openPalette();
    await user.type(input, '刷新快照');
    await user.keyboard('{Enter}');
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([input]) =>
          String(input).includes('/api/workbench?refresh=true'),
        ),
      ).toBe(true),
    );
    expect(
      screen.queryByRole('dialog', { name: '动作面板' }),
    ).not.toBeInTheDocument();
  });

  it('发布策略只预览：回车带人到策略页的待处理队列，全程不发写请求', async () => {
    const { user, dialog, input } = await openPalette();
    await user.type(input, '发布策略');
    expect(within(dialog).getByText('执行预览')).toBeInTheDocument();
    await waitFor(() =>
      expect(
        fetchMock.mock.calls.some(([input]) =>
          String(input).startsWith('/api/search'),
        ),
      ).toBe(true),
    );
    const before = fetchMock.mock.calls.length;
    await user.keyboard('{Enter}');
    expect(location()).toEqual({
      view: 'policy',
      project: null,
      panel: null,
      item: 'desired',
    });
    expect(screen.getByRole('button', { name: '策略' })).toHaveAttribute(
      'aria-current',
      'page',
    );
    // 之后只有策略页自己挂载时的读取：全程没有任何写请求，更没有发布或吊销接口。
    await waitFor(() =>
      expect(fetchMock.mock.calls.length).toBeGreaterThan(before),
    );
    expect(
      fetchMock.mock.calls.every(
        ([, init]) => (init?.method ?? 'GET') === 'GET',
      ),
    ).toBe(true);
    expect(
      fetchMock.mock.calls.some(([input]) =>
        /\/api\/v1\/(policy\/activate|agents\/revoke)/.test(String(input)),
      ),
    ).toBe(false);
  });

  it('跳到黑板任务：来自跨项目汇总，选中并进拓扑', async () => {
    route = (path) =>
      path.startsWith('/api/v1/board/overview')
        ? jsonResponse(
            overviewResponse([
              {
                id: taskId('A'),
                title: '前端全面重设计',
                project: '项目/星舟',
                status: 'queued',
              },
            ]),
          )
        : undefined;
    const { user, input, list } = await openPalette();
    await user.type(input, '重设计');
    const option = await within(list).findByRole('option', {
      name: '协作 › 前端全面重设计',
    });
    expect(option).toHaveAttribute('aria-selected', 'true');
    await user.keyboard('{Enter}');
    expect(location()).toEqual({
      view: 'collab',
      project: null,
      panel: null,
      item: taskId('A'),
    });
  });
});

describe('只在入口真实可用时收录', () => {
  it('没有内存草稿、文件操作未装配时，不列草稿、最近删除与删除文件', async () => {
    const { user, input, list } = await openPalette();
    await user.type(input, '草稿');
    expect(
      within(list).queryByRole('option', { name: '打开内存草稿' }),
    ).not.toBeInTheDocument();
    await user.clear(input);
    await user.type(input, '删除');
    expect(
      within(list).queryByRole('option', { name: '打开最近删除' }),
    ).not.toBeInTheDocument();
    expect(
      within(list).queryByRole('option', { name: '删除文件' }),
    ).not.toBeInTheDocument();
  });

  it('有内存草稿时可打开草稿列表', async () => {
    const drafts = new DraftStore().forScope(ownerScope);
    drafts.keep({ base: canonical, draft: '未保存的改动', pending: null });
    const { user, input } = await openPalette(
      <DraftContext.Provider value={drafts}>
        <App />
      </DraftContext.Provider>,
    );
    await user.type(input, '草稿');
    await user.keyboard('{Enter}');
    expect(
      await screen.findByRole('dialog', { name: '本页内存草稿' }),
    ).toBeInTheDocument();
  });

  it('文件操作可用时可打开最近删除，删除文件只预览', async () => {
    route = (path) => {
      if (path === '/api/v1/file-operations')
        return jsonResponse({
          enabled: true,
          vault_uuid: canonical.vault_uuid,
        });
      if (path.startsWith('/api/v1/tombstones?'))
        return jsonResponse({ entries: [], next_cursor: null });
      return undefined;
    };
    const store = new FileOperationStore().forScope(ownerScope);
    const ui = (
      <FileOperationContext.Provider value={store}>
        <App />
      </FileOperationContext.Provider>
    );
    const user = userEvent.setup();
    render(ui);
    await screen.findByRole('table');
    // 顶栏入口出现即文件操作接口已核对可用。
    await screen.findByRole('button', { name: '最近删除' });
    await user.click(screen.getByRole('button', { name: /^动作面板/ }));
    const dialog = screen.getByRole('dialog', { name: '动作面板' });
    const input = within(dialog).getByRole('combobox', {
      name: '跳转、搜索或执行动作',
    });
    await user.type(input, '删除');
    const list = within(dialog).getByRole('listbox');
    expect(
      within(list).getByRole('option', { name: '删除文件' }),
    ).toBeInTheDocument();
    await user.click(
      within(list).getByRole('option', { name: '打开最近删除' }),
    );
    expect(
      await screen.findByRole('dialog', { name: '最近删除' }),
    ).toBeInTheDocument();
    expect(
      await screen.findByText('这一页没有可恢复的删除记录。'),
    ).toBeInTheDocument();
  });
});
