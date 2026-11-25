import {
  act,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { presenceBody } from '../../../dev/fixtures/presence';
import { PresenceSection } from '../../cockpit/PresenceSection';
import {
  PRESENCE_ROWS,
  presenceAt,
  presenceDeadline,
  presenceGroup,
} from '../../cockpit/presence';
import {
  PRESENCE_READ_TIMEOUT_MS,
  usePresenceRead,
} from '../../cockpit/usePresenceRead';
import { AUTO_REFRESH_READ_TIMEOUT_MS } from '../../live/autoRefresh';
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
        state={{ kind: 'ready', presence: ready(many), readAt: NOW }}
        onRetry={() => {}}
      />,
    );
    expect(section()).toHaveTextContent('另 3 条没列出');
  });

  it('只有「等你」标红；说明写清只含认得准的会话', () => {
    render(
      <PresenceSection
        state={{ kind: 'ready', presence: ready(), readAt: NOW }}
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
        state={{ kind: 'ready', presence: ready([]), readAt: NOW }}
        onRetry={() => {}}
      />,
    );
    expect(section()).toHaveTextContent('现在没有推来的会话状态');
    expect(within(section()).queryByRole('list')).toBeNull();
  });

  it('「从几点起攒」跨天就带上日期', () => {
    const since = (iso: string) => {
      const body = { ...presenceBody(NOW), since: iso };
      const { unmount } = render(
        <PresenceSection
          state={{ kind: 'ready', presence: parsePresence(body), readAt: NOW }}
          onRetry={() => {}}
        />,
      );
      const text = section().textContent ?? '';
      unmount();
      return /从 (.+) 起攒/.exec(text)?.[1];
    };
    expect(since(new Date(NOW - 60_000).toISOString())).toMatch(
      /^\d{2}:\d{2}$/,
    );
    const day = 24 * 3_600_000;
    expect(since(new Date(NOW - day).toISOString())).toMatch(
      /^昨天 \d{2}:\d{2}$/,
    );
    expect(since(new Date(NOW - 3 * day).toISOString())).toMatch(
      /^\d+月\d+日 \d{2}:\d{2}$/,
    );
  });

  it('到期：各条按自己的岁数去掉，整份太久没重读就是不知道', () => {
    const rows = presenceBody(NOW).sessions;
    const presence = ready([
      { ...rows[0], age_secs: 290 },
      { ...rows[1], age_secs: 12 },
    ]);
    expect(presenceDeadline(presence, NOW)).toBe(NOW + 10_000);
    expect(presenceAt(presence, NOW, NOW + 9_999)).toBe(presence);
    const left = presenceAt(presence, NOW, NOW + 10_000);
    expect(left?.sessions).toEqual([presence.sessions[1]]);
    expect(presenceDeadline(left!, NOW)).toBe(NOW + 288_000);
    // 原来有的全到期了：不知道现在有没有，不说成「没有」。
    expect(presenceAt(left!, NOW, NOW + 287_999)).toBe(left);
    expect(presenceAt(left!, NOW, NOW + 288_000)).toBeNull();
    const together = ready([
      { ...rows[0], age_secs: 25 },
      { ...rows[1], age_secs: 25 },
    ]);
    expect(presenceAt(together, NOW, NOW + 275_000)).toBeNull();
    // 一条没有的也会过期：太久没重读，不能一直说「没有」。
    const empty = ready([]);
    expect(presenceDeadline(empty, NOW)).toBe(NOW + 300_000);
    expect(presenceAt(empty, NOW, NOW + 299_999)).toBe(empty);
    expect(presenceAt(empty, NOW, NOW + 300_000)).toBeNull();
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

describe('本机会话的时限', () => {
  const fetchMock = vi.fn<typeof fetch>();
  let reply: () => Response | Promise<Response>;
  const never = () => new Promise<Response>(() => {});
  beforeEach(() => {
    vi.useFakeTimers({ now: NOW });
    fetchMock.mockReset();
    fetchMock.mockImplementation(async () => reply());
    vi.stubGlobal('fetch', fetchMock);
  });
  afterEach(() => {
    vi.useRealTimers();
  });
  const pass = (ms: number) =>
    act(async () => {
      await vi.advanceTimersByTimeAsync(ms);
    });
  const start = async () => {
    const hook = renderHook(() => usePresenceRead());
    await pass(0);
    return hook.result;
  };

  it('自己的时限比自动刷新那一轮短', () => {
    expect(PRESENCE_READ_TIMEOUT_MS).toBeLessThan(AUTO_REFRESH_READ_TIMEOUT_MS);
  });

  it('首次读取挂着不回：到点换成读不到，下一轮安静重读能救回来', async () => {
    reply = never;
    const result = await start();
    await pass(PRESENCE_READ_TIMEOUT_MS - 1);
    expect(result.current.state.kind).toBe('loading');
    await pass(1);
    expect(result.current.state).toEqual({
      kind: 'error',
      message: '本机会话读取超时。',
    });
    reply = () => jsonResponse(presenceBody(NOW));
    await act(async () => {
      await result.current.revalidate(new AbortController().signal);
    });
    expect(result.current.state.kind).toBe('ready');
  });

  it('安静重读挂着不回：到点换成读不到，底下的请求叫停', async () => {
    reply = () => jsonResponse(presenceBody(NOW));
    const result = await start();
    expect(result.current.state.kind).toBe('ready');
    reply = never;
    let pending: Promise<unknown> | undefined;
    act(() => {
      pending = result.current.revalidate(new AbortController().signal);
    });
    await pass(PRESENCE_READ_TIMEOUT_MS);
    expect(await pending).toEqual({ ok: true });
    expect(result.current.state).toEqual({
      kind: 'error',
      message: '本机会话读取超时。',
    });
    const signal = fetchMock.mock.calls.at(-1)?.[1]?.signal;
    expect(signal?.aborted).toBe(true);
  });

  it('读取总被中止、一直没有结果：到期的自己消失，满五分钟换成读不到', async () => {
    const rows = presenceBody(NOW).sessions;
    reply = () =>
      jsonResponse({
        ...presenceBody(NOW),
        sessions: [
          { ...rows[0], age_secs: 280 },
          { ...rows[1], age_secs: 10 },
        ],
      });
    const result = await start();
    const states = () =>
      result.current.state.kind === 'ready'
        ? result.current.state.presence.sessions.map(({ state }) => state)
        : result.current.state.kind;
    expect(states()).toEqual(['waiting', 'running']);
    reply = never;
    const aborted = new AbortController();
    act(() => {
      void result.current.revalidate(aborted.signal);
    });
    aborted.abort();
    // 自动刷新叫停，底下的请求也跟着停。
    expect(fetchMock.mock.calls.at(-1)?.[1]?.signal?.aborted).toBe(true);
    await pass(19_999);
    expect(states()).toEqual(['waiting', 'running']);
    await pass(1);
    expect(states()).toEqual(['running']);
    await pass(269_999);
    expect(states()).toEqual(['running']);
    await pass(1);
    expect(result.current.state).toEqual({
      kind: 'error',
      message: '太久没读到新的状态。',
    });
  });

  it('系统钟往回拨：剩下的最多再留一份的有效期，不会留到拨回来的钟走到那一刻', async () => {
    const rows = presenceBody(NOW).sessions;
    reply = () =>
      jsonResponse({
        ...presenceBody(NOW),
        sessions: [
          { ...rows[0], age_secs: 280 },
          { ...rows[1], age_secs: 10 },
        ],
      });
    const result = await start();
    vi.setSystemTime(NOW - 3_600_000);
    // 第一条到期，定时器照拨过的钟重挂。
    await pass(20_000);
    expect(result.current.state).toMatchObject({
      kind: 'ready',
      presence: { sessions: [{ state: 'running' }] },
    });
    await pass(300_000);
    expect(result.current.state.kind).toBe('error');
  });

  it('睡了很久醒来：重读一开始就先收掉过期的', async () => {
    reply = () => jsonResponse(presenceBody(NOW));
    const result = await start();
    // 睡眠时定时器不走，钟在走。
    vi.setSystemTime(NOW + 3_600_000);
    reply = never;
    act(() => {
      void result.current.revalidate(new AbortController().signal);
    });
    expect(result.current.state.kind).toBe('error');
  });
});
