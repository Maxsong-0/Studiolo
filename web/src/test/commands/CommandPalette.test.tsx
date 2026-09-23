import { useState } from 'react';
import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { COMMAND_TARGETS } from '../../commands/focusTarget';
import type { CommandContext } from '../../commands/registry';
import { CommandPalette } from '../../components/CommandPalette';
import type { SearchHit } from '../../types';
import { jsonResponse } from '../fixtures';
import { overviewResponse, paletteContext, taskId } from './paletteFixtures';

type Route = (url: URL) => Response | Promise<Response>;

const fetchMock = vi.fn<typeof fetch>();
let overviewRoute: Route;
let searchRoute: Route;

function hits(count: number): SearchHit[] {
  return Array.from({ length: count }, (_, index) => ({
    path: `项目/星舟/笔记/第${index + 1}篇.md`,
    title: `第${index + 1}篇笔记`,
    line: index + 1,
    snippet: '',
    kind: 'path' as const,
  }));
}

beforeEach(() => {
  overviewRoute = () =>
    jsonResponse({ code: 'not_found', error: '黑板未启用。' }, 404);
  searchRoute = () =>
    jsonResponse({ generation: 7, hits: hits(1), truncated: false });
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/v1/board/overview') return overviewRoute(url);
    if (url.pathname === '/api/search') return searchRoute(url);
    return jsonResponse({ error: '未知' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
});

function searches() {
  return fetchMock.mock.calls
    .map(([input]) => new URL(String(input), 'http://localhost'))
    .filter((url) => url.pathname === '/api/search');
}

/** 入口按钮＋面板；先聚焦入口再打开，焦点归还才有落点。 */
async function openPalette(overrides: Partial<CommandContext> = {}) {
  const { context, actions } = paletteContext(overrides);
  const onClose = vi.fn();
  const onRefresh = vi.fn();
  function Harness() {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button onClick={() => setOpen(true)}>入口</button>
        {open ? (
          <CommandPalette
            context={context}
            generation={7}
            onClose={() => {
              onClose();
              setOpen(false);
            }}
            onRefresh={onRefresh}
          />
        ) : null}
      </>
    );
  }
  const user = userEvent.setup();
  render(<Harness />);
  const trigger = screen.getByRole('button', { name: '入口' });
  await user.click(trigger);
  const dialog = screen.getByRole('dialog', { name: '动作面板' });
  const input = within(dialog).getByRole('combobox', {
    name: '跳转、搜索或执行动作',
  });
  return { user, dialog, input, trigger, actions, onClose, onRefresh };
}

/** 结果列表里的行（不含范围下拉框自己的 option）。 */
function rows(dialog: HTMLElement) {
  return within(within(dialog).getByRole('listbox')).getAllByRole('option');
}

function selected(dialog: HTMLElement) {
  return rows(dialog).find(
    (option) => option.getAttribute('aria-selected') === 'true',
  );
}

describe('动作面板：键盘与指针', () => {
  it('↑↓ 循环、Home/End 到首末，焦点始终在输入框；↵ 执行选中项并关面板', async () => {
    const { user, dialog, input, actions, onClose } = await openPalette();
    expect(input).toHaveFocus();
    const options = rows(dialog);
    expect(options[0]).toHaveAccessibleName('打开状态详情');
    expect(selected(dialog)).toBe(options[0]);
    expect(input).toHaveAttribute('aria-activedescendant', options[0].id);
    await user.keyboard('{ArrowUp}');
    expect(selected(dialog)).toBe(options.at(-1));
    await user.keyboard('{Home}');
    expect(selected(dialog)).toBe(options[0]);
    await user.keyboard('{End}');
    expect(selected(dialog)).toBe(options.at(-1));
    await user.keyboard('{ArrowDown}');
    expect(selected(dialog)).toBe(options[0]);
    await user.keyboard('{ArrowDown}');
    expect(selected(dialog)).toHaveAccessibleName('刷新快照');
    expect(input).toHaveFocus();
    // ⌘↵ 没有「新格」可开：什么也不做。
    await user.keyboard('{Meta>}{Enter}{/Meta}');
    expect(actions.refreshSnapshot).not.toHaveBeenCalled();
    await user.keyboard('{Enter}');
    expect(actions.refreshSnapshot).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it('分组标题按动作、跳转排列；匹配字用红线标出；说明作描述', async () => {
    const { user, dialog, input } = await openPalette();
    expect(
      within(dialog)
        .getAllByRole('group')
        .map((group) => group.getAttribute('aria-labelledby'))
        .map((id) => document.getElementById(id!)?.textContent),
    ).toEqual(['动作', '跳转']);
    await user.type(input, '快照');
    const option = within(dialog).getByRole('option', { name: '刷新快照' });
    expect(option.querySelector('mark.palette-match')).toHaveTextContent(
      '快照',
    );
    expect(option).toHaveAccessibleDescription('重新读取工作台');
    expect(within(dialog).getByText(/^\d+ 条结果$/)).toBeInTheDocument();
  });

  it('指针移过即选中，点击执行；按下不抢走输入框的焦点', async () => {
    const { user, dialog, input, actions } = await openPalette();
    const option = within(dialog).getByRole('option', {
      name: '协作 › 时间线',
    });
    fireEvent.pointerMove(option);
    expect(option).toHaveAttribute('aria-selected', 'true');
    expect(fireEvent.mouseDown(option)).toBe(false);
    expect(input).toHaveFocus();
    await user.click(option);
    expect(actions.goTo).toHaveBeenCalledWith({
      view: 'collab',
      panel: 'timeline',
      project: null,
    });
    expect(actions.focusTarget).not.toHaveBeenCalled();
  });

  it('Esc 关闭并把焦点还给入口按钮', async () => {
    const { dialog, trigger, onClose } = await openPalette();
    fireEvent(dialog, new Event('cancel', { cancelable: true }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(trigger).toHaveFocus();
  });
});

describe('不可逆动作只预览', () => {
  it('选中时显示红色虚线框「执行预览」；回车只带人去策略页并聚焦待处理队列，不发任何请求', async () => {
    const { user, dialog, input, actions } = await openPalette();
    await user.type(input, '发布');
    const option = within(dialog).getByRole('option', { name: '发布策略' });
    expect(option).toHaveAttribute('aria-selected', 'true');
    expect(within(option).getByText('↵')).toBeInTheDocument();
    const preview = within(dialog)
      .getByText('执行预览')
      .closest('.palette-preview') as HTMLElement;
    expect(preview).toHaveTextContent(
      '发布会把一份已校验通过的规则草案（desired）变成新的生效版本（effective）。',
    );
    expect(preview).toHaveTextContent(
      '按 ↵ 只会前往「策略 › 待处理队列」，面板不发任何请求；确认仍在那里完成。',
    );
    expect(option.getAttribute('aria-describedby')?.split(' ')).toContain(
      preview.id,
    );
    // 等文档组的搜索落定，再数执行前后的请求。
    await within(dialog).findByRole('option', { name: '第1篇笔记' });
    const before = fetchMock.mock.calls.length;
    await user.keyboard('{Enter}');
    expect(fetchMock.mock.calls.length).toBe(before);
    expect(
      fetchMock.mock.calls.every(
        ([, init]) => (init?.method ?? 'GET') === 'GET',
      ),
    ).toBe(true);
    expect(actions.goTo).toHaveBeenCalledWith({
      view: 'policy',
      item: 'desired',
    });
    expect(actions.focusTarget).toHaveBeenCalledWith(
      COMMAND_TARGETS.policyQueue,
    );
    for (const action of [
      'refreshSnapshot',
      'openSource',
      'openDrafts',
      'openRecentlyDeleted',
      'openStatus',
    ] as const)
      expect(actions[action]).not.toHaveBeenCalled();
  });

  it('没选中时只在行尾标红字「预览」，不显示预览框', async () => {
    const { user, dialog, input } = await openPalette({
      fileOperations: { deletable: true },
    });
    await user.type(input, '预览');
    const rows = within(
      within(dialog).getByRole('group', { name: '动作' }),
    ).getAllByRole('option');
    expect(rows.map((row) => row.getAttribute('aria-label'))).toEqual([
      '发布策略',
      '回滚策略',
      '删除文件',
      '落地候选产物',
    ]);
    expect(within(rows[1]).getByText('预览')).toHaveClass('palette-option-tag');
    await user.keyboard('{ArrowDown}');
    expect(
      within(dialog).getByText('执行预览').closest('.palette-preview'),
    ).toHaveTextContent('回滚会把选中的历史版本重新发布为新的生效版本');
  });
});

describe('文档组', () => {
  it('不足两个字不搜；停手才发一次；只列前 5 条并说明命中总数', async () => {
    searchRoute = () =>
      jsonResponse({ generation: 7, hits: hits(7), truncated: false });
    const { user, dialog, input } = await openPalette();
    await user.type(input, 'x');
    expect(
      within(dialog).getByText('输入 2 个字以上，同时搜索文档正文与路径。'),
    ).toBeInTheDocument();
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(searches()).toHaveLength(0);
    await user.type(input, 'yz');
    await within(dialog).findByRole('option', { name: '第1篇笔记' });
    expect(searches()).toHaveLength(1);
    expect(searches()[0].searchParams.get('q')).toBe('xyz');
    expect(searches()[0].searchParams.get('mode')).toBe('full_text');
    const documents = within(
      within(dialog).getByRole('group', { name: '文档' }),
    ).getAllByRole('option');
    expect(
      documents.map((option) => option.getAttribute('aria-label')),
    ).toEqual([
      '第1篇笔记',
      '第2篇笔记',
      '第3篇笔记',
      '第4篇笔记',
      '第5篇笔记',
    ]);
    expect(documents[0]).toHaveAccessibleDescription(
      '项目/星舟/笔记/第1篇.md · 第 1 行',
    );
    expect(
      within(dialog).getByText(
        '文档命中 7 条，这里列前 5 条；完整结果请到「文档」视图搜索。',
      ),
    ).toBeInTheDocument();
  });

  it('选中文档回车交给 openSource；范围选了项目就带上 project', async () => {
    const { user, dialog, input, actions } = await openPalette();
    await user.selectOptions(
      within(dialog).getByRole('combobox', { name: '文档搜索范围' }),
      'xingzhou',
    );
    await user.type(input, '原文正文');
    const option = await within(dialog).findByRole('option', {
      name: '第1篇笔记',
    });
    expect(option).toHaveAttribute('aria-selected', 'true');
    expect(searches().at(-1)?.searchParams.get('project')).toBe('xingzhou');
    input.focus();
    await user.keyboard('{Enter}');
    expect(actions.openSource).toHaveBeenCalledWith({
      path: '项目/星舟/笔记/第1篇.md',
      line: 1,
    });
  });

  it('快照已更新（409）给「刷新工作台」', async () => {
    searchRoute = () => jsonResponse({ error: '搜索结果来自旧快照。' }, 409);
    const { user, dialog, input, onRefresh } = await openPalette();
    await user.type(input, '原文');
    const stale = await within(dialog).findByRole('alert');
    expect(stale).toHaveTextContent(
      '快照已更新，请刷新工作台。搜索结果来自旧快照。',
    );
    await user.click(within(stale).getByRole('button', { name: '刷新工作台' }));
    expect(onRefresh).toHaveBeenCalledTimes(1);
  });

  it('服务忙（429）可重试，重试会重新发请求', async () => {
    searchRoute = () => jsonResponse({ error: '稍后再试。' }, 429);
    const { user, dialog, input } = await openPalette();
    await user.type(input, '原文');
    const busy = await within(dialog).findByRole('alert');
    expect(busy).toHaveTextContent('搜索服务正忙，请稍后重试。稍后再试。');
    const count = searches().length;
    searchRoute = () =>
      jsonResponse({ generation: 7, hits: hits(1), truncated: false });
    await user.click(within(busy).getByRole('button', { name: '重试搜索' }));
    await within(dialog).findByRole('option', { name: '第1篇笔记' });
    expect(searches()).toHaveLength(count + 1);
  });

  it('没有任何命中时说明，结果数为 0', async () => {
    searchRoute = () =>
      jsonResponse({ generation: 7, hits: [], truncated: false });
    const { user, dialog, input } = await openPalette();
    await user.type(input, 'zzqq');
    expect(
      await within(dialog).findByText('没有匹配的文档，请换个词或调整范围。'),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText('没有匹配的动作或跳转。'),
    ).toBeInTheDocument();
    expect(within(dialog).getByText('0 条结果')).toBeInTheDocument();
    expect(input).toHaveAttribute('aria-expanded', 'false');
    expect(input).not.toHaveAttribute('aria-activedescendant');
  });
});

describe('黑板任务', () => {
  it('读到汇总后列未结束的任务，回车选中并进拓扑', async () => {
    overviewRoute = () =>
      jsonResponse(
        overviewResponse([
          {
            id: taskId('A'),
            title: '前端全面重设计',
            project: '项目/星舟',
            status: 'queued',
          },
          {
            id: taskId('B'),
            title: '已经收尾的盘点',
            project: '项目/星舟',
            status: 'succeeded',
          },
        ]),
      );
    const { user, dialog, input, actions } = await openPalette();
    await user.type(input, '重设计');
    const option = await within(dialog).findByRole('option', {
      name: '协作 › 前端全面重设计',
    });
    expect(option).toHaveAccessibleDescription('星舟 · 待认领');
    expect(
      within(dialog).queryByRole('option', { name: /已经收尾/ }),
    ).not.toBeInTheDocument();
    await user.keyboard('{Enter}');
    expect(actions.goTo).toHaveBeenCalledWith({
      view: 'collab',
      item: taskId('A'),
    });
    const overviewCalls = fetchMock.mock.calls.filter(([input]) =>
      String(input).startsWith('/api/v1/board/overview'),
    );
    expect(overviewCalls.map(([input]) => String(input))).toEqual([
      '/api/v1/board/overview?window_hours=24',
    ]);
  });

  it('黑板未启用（404）只说一句，不算错误', async () => {
    const { dialog } = await openPalette();
    expect(
      await within(dialog).findByText('本机未启用黑板，没有黑板任务可跳。'),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByText(/黑板任务读取失败/),
    ).not.toBeInTheDocument();
  });

  it('汇总读不到或形状不对：说明原因，不当成空黑板，其余命令照常', async () => {
    overviewRoute = () => jsonResponse({ items: [] });
    const { dialog } = await openPalette();
    expect(
      await within(dialog).findByText(
        '黑板任务读取失败：黑板汇总格式无效，实时状态不可确认。',
      ),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByRole('option', { name: '刷新快照' }),
    ).toBeInTheDocument();
  });
});
