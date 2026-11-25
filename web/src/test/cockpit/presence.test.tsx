import {
  act,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { presenceBody } from '../../../dev/fixtures/presence';
import { PresenceSection } from '../../cockpit/PresenceSection';
import { PRESENCE_ROWS, presenceGroup } from '../../cockpit/presence';
import { usePresenceRead } from '../../cockpit/usePresenceRead';
import { parsePresence, type PresenceSession } from '../../presenceModel';
import { jsonResponse } from '../fixtures';

const NOW = Date.parse('2026-11-24T20:31:00Z');
const ready = (sessions?: unknown[]) => {
  const body = presenceBody(NOW);
  return parsePresence(sessions ? { ...body, sessions } : body);
};
const section = () => screen.getByRole('region', { name: '本机会话' });

describe('本机会话一组', () => {
  it('等你的排最前，写清项目、分支、做到第几条、最近动作、多久没动', () => {
    const group = presenceGroup(ready());
    expect(group).toMatchObject({ running: 1, waiting: 1, hidden: 0 });
    expect(group.rows.map((row) => [row.state, row.status])).toEqual([
      ['waiting', '等你'],
      ['running', '在跑'],
      ['idle', '做完或等你回话'],
    ]);
    expect(group.rows[0]).toMatchObject({
      vendor: 'Claude Code',
      where: 'HomeLab-Center-Docs · claude/t-092-coding',
      detail: '第 3/8 条 · 问人 · 4 分钟没动',
    });
    expect(group.rows[1].detail).toBe('第 5 条 · 在改');
    // 没带的一项都不写，不写「未知」。
    expect(group.rows[2]).toMatchObject({
      vendor: 'Codex',
      where: 'Studiolo-Docs',
      detail: '',
    });
  });

  it('多久没动按分钟、小时写', () => {
    const detail = (idle_secs: number) =>
      presenceGroup(ready([{ ...presenceBody(NOW).sessions[2], idle_secs }]))
        .rows[0].detail;
    expect(detail(59)).toBe('');
    expect(detail(60)).toBe('1 分钟没动');
    expect(detail(3599)).toBe('59 分钟没动');
    expect(detail(7200)).toBe('2 小时没动');
  });

  it('太多时只列前几条，其余写条数；数照全部的算', () => {
    const many = Array.from({ length: PRESENCE_ROWS + 3 }, () => ({
      ...presenceBody(NOW).sessions[1],
    }));
    const group = presenceGroup(ready(many));
    expect(group.rows).toHaveLength(PRESENCE_ROWS);
    expect(group).toMatchObject({ hidden: 3, running: PRESENCE_ROWS + 3 });
    expect(new Set(group.rows.map((row) => row.key)).size).toBe(PRESENCE_ROWS);
    render(
      <PresenceSection
        state={{ kind: 'ready', presence: ready(many) }}
        onRetry={() => {}}
      />,
    );
    expect(section()).toHaveTextContent('另 3 条没列出');
  });

  it('只有「等你」标红；说明写清只含认得准的会话', () => {
    render(
      <PresenceSection
        state={{ kind: 'ready', presence: ready() }}
        onRetry={() => {}}
      />,
    );
    const rows = within(section()).getAllByRole('listitem');
    expect(rows.map((row) => row.dataset.state)).toEqual([
      'waiting',
      'running',
      'idle',
    ]);
    expect(section()).toHaveTextContent('1 个在跑 · 1 个等你');
    expect(section()).toHaveTextContent('只含认得准是你自己开的会话');
    expect(section()).toHaveTextContent('5 分钟没有新状态就不显示');
    expect(section()).not.toHaveTextContent('未知');
  });

  it('没有会话：写清是现在没有，不列空表', () => {
    render(
      <PresenceSection
        state={{ kind: 'ready', presence: ready([]) }}
        onRetry={() => {}}
      />,
    );
    expect(section()).toHaveTextContent('现在没有推来的会话状态');
    expect(within(section()).queryByRole('list')).toBeNull();
  });

  it('读不到：写原因、给重试；没装黑板：整组不出现', async () => {
    const retry = vi.fn();
    const { rerender } = render(
      <PresenceSection
        state={{ kind: 'error', message: '黑板暂不可用，请重试。' }}
        onRetry={retry}
      />,
    );
    expect(section()).toHaveTextContent('本机会话读不到：黑板暂不可用');
    await userEvent.click(screen.getByRole('button', { name: '重试本机会话' }));
    expect(retry).toHaveBeenCalledTimes(1);
    rerender(<PresenceSection state={{ kind: 'disabled' }} onRetry={retry} />);
    expect(screen.queryByRole('region', { name: '本机会话' })).toBeNull();
  });
});

describe('本机会话的读取', () => {
  const fetchMock = vi.fn<typeof fetch>();
  let reply: () => Response | Promise<Response>;
  beforeEach(() => {
    fetchMock.mockReset();
    fetchMock.mockImplementation(async () => reply());
    vi.stubGlobal('fetch', fetchMock);
    reply = () => jsonResponse(presenceBody(NOW));
  });
  const sessions = (read: ReturnType<typeof usePresenceRead>) =>
    read.state.kind === 'ready'
      ? read.state.presence.sessions.map(
          (session: PresenceSession) => session.state,
        )
      : null;

  it('安静重读没读成就换成「读不到」，不留旧状态；自动刷新那一轮不算没读成', async () => {
    const { result } = renderHook(() => usePresenceRead());
    await waitFor(() => expect(result.current.state.kind).toBe('ready'));
    expect(sessions(result.current)).toEqual(['waiting', 'running', 'idle']);
    reply = () => jsonResponse({ error: '黑板暂不可用，请重试。' }, 503);
    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.revalidate(new AbortController().signal);
    });
    expect(outcome).toEqual({ ok: true });
    expect(result.current.state).toEqual({
      kind: 'error',
      message: '黑板暂不可用，请重试。',
    });
    // 下一轮读成了就回来。
    reply = () => jsonResponse(presenceBody(NOW));
    await act(async () => {
      await result.current.revalidate(new AbortController().signal);
    });
    expect(result.current.state.kind).toBe('ready');
  });

  it('格式不符当读不到；404 当没装黑板，之后不再安静重读', async () => {
    reply = () => jsonResponse({ sessions: [] });
    const first = renderHook(() => usePresenceRead());
    await waitFor(() => expect(first.result.current.state.kind).toBe('error'));
    reply = () => jsonResponse({ error: '黑板未配置。' }, 404);
    const second = renderHook(() => usePresenceRead());
    await waitFor(() =>
      expect(second.result.current.state.kind).toBe('disabled'),
    );
    const calls = fetchMock.mock.calls.length;
    await act(async () => {
      await second.result.current.revalidate(new AbortController().signal);
    });
    expect(fetchMock.mock.calls.length).toBe(calls);
  });

  it('手动重读显示读取中；中止的应答不写回', async () => {
    const { result } = renderHook(() => usePresenceRead());
    await waitFor(() => expect(result.current.state.kind).toBe('ready'));
    act(() => result.current.reload());
    expect(result.current.state.kind).toBe('loading');
    await waitFor(() => expect(result.current.state.kind).toBe('ready'));
    const aborted = new AbortController();
    reply = () => {
      aborted.abort();
      return jsonResponse({ error: '晚到的' }, 503);
    };
    await act(async () => {
      await result.current.revalidate(aborted.signal);
    });
    expect(result.current.state.kind).toBe('ready');
  });
});
