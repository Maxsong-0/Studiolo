/*
 * 四路读取的「安静重读」（T-196）：自动刷新每轮各调一次。
 * 不改成读取中；读成了换数，没读成不动数、把原因交回；手动读取在途时让路，重读途中手动读取抢先则结果作废。
 */
import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useCockpitReads } from '../../cockpit/useCockpitReads';
import { useOverviewRead } from '../../cockpit/useOverviewRead';
import { usePolicyPendingRead } from '../../cockpit/usePolicyPendingRead';
import type { Revalidate, Revalidation } from '../../live/autoRefresh';
import type { Workbench } from '../../types';
import { useWorkbench } from '../../useWorkbench';
import { overviewResponse, taskId } from '../commands/paletteFixtures';
import { fixture, jsonResponse } from '../fixtures';
import { desiredBody, listBody } from '../policy/policyFixtures';
import { deferred } from '../sessionFixtures';

const WORKBENCH = '/api/workbench';
const OVERVIEW = '/api/v1/board/overview';
const DESIRED = '/api/v1/policy/desired';
const HEALTH = '/api/v1/health';

type Reply = () => Response | Promise<Response>;
const fetchMock = vi.fn<typeof fetch>();
/** 每例按路径前缀准备应答；没准备的路径直接报错，免得多发了请求却没人发现。 */
let replies: Record<string, Reply>;

beforeEach(() => {
  replies = {};
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = String(input);
    const prefix = Object.keys(replies).find((key) => path.startsWith(key));
    if (!prefix) throw new Error(`没有准备应答：${path}`);
    return replies[prefix]();
  });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  vi.useRealTimers();
});

/** 发往某个接口的请求，按先后。 */
const requests = (prefix: string) =>
  fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((path) => path.startsWith(prefix));

/** 安静重读一次，等它读完。 */
async function quiet(revalidate: Revalidate): Promise<Revalidation> {
  let outcome!: Revalidation;
  await act(async () => {
    outcome = await revalidate(new AbortController().signal);
  });
  return outcome;
}

/** 发起一次安静重读但不等它：应答由测试稍后放行。 */
function startQuiet(revalidate: Revalidate, signal: AbortSignal) {
  let outcome!: Promise<Revalidation>;
  act(() => {
    outcome = revalidate(signal);
  });
  return outcome;
}

describe('工作台快照的安静重读', () => {
  /** 挂上钩子，并记下每次重画时是不是「读取中」。 */
  function mount() {
    const loadingSeen: boolean[] = [];
    const view = renderHook(() => {
      const workbench = useWorkbench();
      loadingSeen.push(workbench.loading);
      return workbench;
    });
    return { ...view, loadingSeen };
  }

  it('不带 refresh=true，全程不显示读取中；内容没变留着原来那个对象，变了才换', async () => {
    let snapshot: Workbench = fixture;
    replies[WORKBENCH] = () => jsonResponse(snapshot);
    const { result, loadingSeen } = mount();
    await waitFor(() => expect(result.current.loading).toBe(false));
    const first = result.current.data;
    expect(first?.generation).toBe(fixture.generation);
    loadingSeen.length = 0;

    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(result.current.data).toBe(first);

    snapshot = { ...fixture, generation: fixture.generation + 1 };
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(result.current.data?.generation).toBe(fixture.generation + 1);

    // 代次没变、别的内容变了，也要换上。
    snapshot = { ...snapshot, source_label: '换了名字的库' };
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(result.current.data?.source_label).toBe('换了名字的库');

    // 首次读取一次、安静重读三次，都不带整库重算的那个参数。
    expect(requests(WORKBENCH)).toEqual([
      WORKBENCH,
      WORKBENCH,
      WORKBENCH,
      WORKBENCH,
    ]);
    expect(loadingSeen).not.toContain(true);
  });

  it('没读成：快照不动、不写错误，把原因交回', async () => {
    let failing = false;
    replies[WORKBENCH] = () =>
      failing
        ? jsonResponse({ error: '副本不可用' }, 503)
        : jsonResponse(fixture);
    const { result, loadingSeen } = mount();
    await waitFor(() => expect(result.current.loading).toBe(false));
    const first = result.current.data;
    loadingSeen.length = 0;

    failing = true;
    expect(await quiet(result.current.revalidate)).toEqual({
      ok: false,
      message: '副本不可用',
    });
    expect(result.current.data).toBe(first);
    expect(result.current.error).toBeNull();
    expect(loadingSeen).not.toContain(true);
  });

  it('读成了清掉上一次手动刷新留下的错误', async () => {
    let failing = false;
    replies[WORKBENCH] = () =>
      failing
        ? jsonResponse({ error: '副本不可用' }, 503)
        : jsonResponse(fixture);
    const { result } = mount();
    await waitFor(() => expect(result.current.loading).toBe(false));
    const first = result.current.data;

    failing = true;
    await act(async () => {
      await result.current.refreshLatest();
    });
    expect(result.current.error).toBe('副本不可用');
    expect(result.current.data).toBe(first);

    failing = false;
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(result.current.error).toBeNull();
    expect(result.current.data).toBe(first);
  });

  it('手动读取在途时让路：不发请求', async () => {
    const pending = deferred<Response>();
    replies[WORKBENCH] = () => pending.promise;
    const { result } = mount();
    expect(result.current.loading).toBe(true);

    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(requests(WORKBENCH)).toHaveLength(1);

    await act(async () => pending.resolve(jsonResponse(fixture)));
    await waitFor(() => expect(result.current.data).not.toBeNull());
  });

  it('重读途中手动刷新抢先，或信号中止：过期的结果不写回', async () => {
    let reply: Reply = () => jsonResponse(fixture);
    replies[WORKBENCH] = () => reply();
    const { result } = mount();
    await waitFor(() => expect(result.current.loading).toBe(false));

    // 一、安静重读在途时点了刷新：以刷新读到的为准。
    const slow = deferred<Response>();
    reply = () => slow.promise;
    const overtaken = startQuiet(
      result.current.revalidate,
      new AbortController().signal,
    );
    reply = () => jsonResponse({ ...fixture, generation: 9 });
    await act(async () => {
      await result.current.refreshLatest();
    });
    expect(result.current.data?.generation).toBe(9);
    await act(async () => {
      slow.resolve(jsonResponse({ ...fixture, generation: 8 }));
      await overtaken;
    });
    expect(await overtaken).toEqual({ ok: true });
    expect(result.current.data?.generation).toBe(9);

    // 二、调度器叫停了这一轮：晚到的结果作废。
    const late = deferred<Response>();
    reply = () => late.promise;
    const controller = new AbortController();
    const stopped = startQuiet(result.current.revalidate, controller.signal);
    controller.abort();
    await act(async () => {
      late.resolve(jsonResponse({ ...fixture, generation: 10 }));
      await stopped;
    });
    expect(await stopped).toEqual({ ok: true });
    expect(result.current.data?.generation).toBe(9);
  });
});

describe('黑板汇总的安静重读', () => {
  const STARSHIP = '项目/星舟';
  const one: Parameters<typeof overviewResponse>[0] = [
    {
      id: taskId('A'),
      title: '验收拓扑缩略',
      project: STARSHIP,
      status: 'needs_review',
    },
  ];
  const two: Parameters<typeof overviewResponse>[0] = [
    ...one,
    {
      id: taskId('B'),
      title: '验收今天的时间线',
      project: STARSHIP,
      status: 'needs_review',
    },
  ];

  /** 挂上钩子，并记下每次重画时的读取状态。 */
  function mount(ready = true) {
    const kinds: string[] = [];
    const view = renderHook(
      (props: { ready: boolean; refreshing: boolean }) => {
        const read = useOverviewRead(props);
        kinds.push(read.state.kind);
        return read;
      },
      { initialProps: { ready, refreshing: false } },
    );
    return { ...view, kinds };
  }

  /** 现在这份汇总里有几条任务；不是「读到了」就是 null。 */
  const count = (read: ReturnType<typeof useOverviewRead>) =>
    read.state.kind === 'ready' ? read.state.overview.items.length : null;

  it('不显示读取中，读成了换上新汇总', async () => {
    let items = one;
    replies[OVERVIEW] = () => jsonResponse(overviewResponse(items));
    const { result, kinds } = mount();
    await waitFor(() => expect(count(result.current)).toBe(1));
    kinds.length = 0;

    items = two;
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(count(result.current)).toBe(2);
    expect(new Set(kinds)).toEqual(new Set(['ready']));
    expect(requests(OVERVIEW)).toHaveLength(2);
  });

  it('没读成：留着上一次的汇总，把原因交回', async () => {
    let failing = false;
    replies[OVERVIEW] = () =>
      failing
        ? jsonResponse({ error: '黑板暂时不可用。' }, 503)
        : jsonResponse(overviewResponse(one));
    const { result } = mount();
    await waitFor(() => expect(count(result.current)).toBe(1));
    const before = result.current.state;

    failing = true;
    expect(await quiet(result.current.revalidate)).toEqual({
      ok: false,
      message: '黑板暂时不可用。',
    });
    expect(result.current.state).toBe(before);
  });

  it('重读时发现本机没装黑板（404）：记「未启用」，不算没读成，之后不再读', async () => {
    let installed = true;
    replies[OVERVIEW] = () =>
      installed
        ? jsonResponse(overviewResponse(one))
        : new Response(null, { status: 404 });
    const { result } = mount();
    await waitFor(() => expect(count(result.current)).toBe(1));

    installed = false;
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(result.current.state).toEqual({ kind: 'disabled' });
    expect(requests(OVERVIEW)).toHaveLength(2);

    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(requests(OVERVIEW)).toHaveLength(2);
  });

  it('还没到该读的时候不读', async () => {
    replies[OVERVIEW] = () => jsonResponse(overviewResponse(one));
    const { result } = mount(false);
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(requests(OVERVIEW)).toHaveLength(0);
  });

  it('手动读取在途时让路；重读途中手动重读抢先，过期的结果不写回', async () => {
    const first = deferred<Response>();
    let reply: Reply = () => first.promise;
    replies[OVERVIEW] = () => reply();
    const { result } = mount();
    expect(result.current.state.kind).toBe('loading');
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(requests(OVERVIEW)).toHaveLength(1);
    await act(async () => first.resolve(jsonResponse(overviewResponse(one))));
    await waitFor(() => expect(count(result.current)).toBe(1));

    const slow = deferred<Response>();
    reply = () => slow.promise;
    const overtaken = startQuiet(
      result.current.revalidate,
      new AbortController().signal,
    );
    reply = () => jsonResponse(overviewResponse(two));
    act(() => result.current.reload());
    await waitFor(() => expect(count(result.current)).toBe(2));
    await act(async () => {
      slow.resolve(jsonResponse(overviewResponse(one)));
      await overtaken;
    });
    expect(await overtaken).toEqual({ ok: true });
    expect(count(result.current)).toBe(2);
  });

  it('手动读取 10 秒没有结果按失败算，不一直转圈', async () => {
    vi.useFakeTimers();
    replies[OVERVIEW] = () => new Promise<Response>(() => {});
    const { result } = mount();
    expect(result.current.state.kind).toBe('loading');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(9_999);
    });
    expect(result.current.state.kind).toBe('loading');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(result.current.state).toEqual({
      kind: 'error',
      message: '黑板汇总读取超时，实时状态不可确认。',
    });
  });
});

describe('策略草案的安静重读', () => {
  const draft = (id: string) =>
    desiredBody({ id: `01ABCDEFGHJKMNPQRSTVWXYZ${id}`, subject: `codex${id}` });

  function mount() {
    const kinds: string[] = [];
    const view = renderHook(() => {
      const read = usePolicyPendingRead({ ready: true, refreshing: false });
      kinds.push(read.state.kind);
      return read;
    });
    return { ...view, kinds };
  }

  const count = (read: ReturnType<typeof usePolicyPendingRead>) =>
    read.state.kind === 'ready' ? read.state.desired.length : null;

  it('不显示读取中，读成了换上新列表；没读成留着上一次的', async () => {
    let reply: Reply = () => jsonResponse(listBody([draft('01')]));
    replies[DESIRED] = () => reply();
    const { result, kinds } = mount();
    await waitFor(() => expect(count(result.current)).toBe(1));
    kinds.length = 0;

    reply = () => jsonResponse(listBody([draft('01'), draft('02')]));
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(count(result.current)).toBe(2);
    const before = result.current.state;

    reply = () => jsonResponse({ error: '策略服务暂时不可用。' }, 503);
    expect(await quiet(result.current.revalidate)).toEqual({
      ok: false,
      message: '策略服务暂时不可用。',
    });
    expect(result.current.state).toBe(before);
    expect(new Set(kinds)).toEqual(new Set(['ready']));
    expect(requests(DESIRED)).toHaveLength(3);
  });

  it('两次安静重读叠在一起：后发的已经换上了，先发的才到，就不要它（它读到的可能是写入之前的列表）', async () => {
    replies[DESIRED] = () => jsonResponse(listBody([draft('01'), draft('02')]));
    const { result } = mount();
    await waitFor(() => expect(count(result.current)).toBe(2));

    // 先发的那一次读到的是写入之前的两份，应答晚到；后发的读到写入之后的一份，先到。
    const early = deferred<Response>();
    const late = deferred<Response>();
    const pending = [early.promise, late.promise];
    replies[DESIRED] = () => pending.shift() ?? late.promise;
    const first = startQuiet(
      result.current.revalidate,
      new AbortController().signal,
    );
    const second = startQuiet(
      result.current.revalidate,
      new AbortController().signal,
    );
    await waitFor(() => expect(requests(DESIRED)).toHaveLength(3));

    await act(async () => {
      late.resolve(jsonResponse(listBody([draft('01')])));
      await second;
    });
    expect(count(result.current)).toBe(1);
    const applied = result.current.state;

    let outcome: Revalidation | undefined;
    await act(async () => {
      early.resolve(jsonResponse(listBody([draft('01'), draft('02')])));
      outcome = await first;
    });
    // 晚到的旧结果不算失败（调度器不必退避），只是不用它。
    expect(outcome).toEqual({ ok: true });
    expect(result.current.state).toBe(applied);
  });

  it('本入口没装策略平面：不读', async () => {
    replies[DESIRED] = () =>
      jsonResponse(
        { code: 'policy_plane_absent', error: '本入口未装配策略平面。' },
        404,
      );
    const { result } = mount();
    await waitFor(() => expect(result.current.state.kind).toBe('absent'));
    expect(await quiet(result.current.revalidate)).toEqual({ ok: true });
    expect(requests(DESIRED)).toHaveLength(1);
  });
});

describe('本机健康的安静重读', () => {
  const report = (state: 'normal' | 'degraded') => ({
    overall: state,
    evidence_complete: true,
    sections: [
      {
        code: 'disk-quota',
        title: '磁盘配额',
        state,
        detail: '占用在预警水位以内',
      },
    ],
  });

  it('不显示读取中，读成了换上新报告；没读成留着上一次的', async () => {
    let reply: Reply = () => jsonResponse(report('normal'));
    replies[HEALTH] = () => reply();
    const kinds: string[] = [];
    const { result } = renderHook(() => {
      const reads = useCockpitReads();
      kinds.push(reads.health.kind);
      return reads;
    });
    await waitFor(() => expect(result.current.health.kind).toBe('ready'));
    kinds.length = 0;

    reply = () => jsonResponse(report('degraded'));
    expect(await quiet(result.current.revalidateHealth)).toEqual({ ok: true });
    const health = result.current.health;
    expect(health.kind === 'ready' && health.report.overall).toBe('degraded');

    reply = () => jsonResponse({ error: '健康页暂时不可用。' }, 503);
    expect(await quiet(result.current.revalidateHealth)).toEqual({
      ok: false,
      message: '健康页暂时不可用。',
    });
    expect(result.current.health).toBe(health);
    expect(new Set(kinds)).toEqual(new Set(['ready']));
    expect(requests(HEALTH)).toHaveLength(3);
  });

  it('手动读取在途时让路', async () => {
    const pending = deferred<Response>();
    replies[HEALTH] = () => pending.promise;
    const { result } = renderHook(() => useCockpitReads());
    expect(result.current.health.kind).toBe('loading');
    expect(await quiet(result.current.revalidateHealth)).toEqual({ ok: true });
    expect(requests(HEALTH)).toHaveLength(1);
    await act(async () => pending.resolve(jsonResponse(report('normal'))));
    await waitFor(() => expect(result.current.health.kind).toBe('ready'));
  });
});
