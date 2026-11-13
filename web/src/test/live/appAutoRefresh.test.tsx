/*
 * 自动刷新接进整页之后（T-196）：驾驶舱和协作拓扑开着，就每 20 秒安静地重读一轮
 * （工作台快照、黑板汇总、策略草案、驾驶舱的本机健康，一个接一个）。
 *
 * 时钟是假的，但跟着真实时间慢慢走（shouldAdvanceTime），这样 findBy、waitFor 照常能用；
 * 大段时间由测试自己拨。所以「到点之前不读」一律留出几秒余量再核对。
 * 一轮里第一路（工作台快照）的请求在定时器触发的同一拍发出，后面几路要等前一路读完：
 * 「没有读」只核对工作台请求数，「读了」用 waitFor 等。
 */
import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../../App';
import { ATTENTION_IDLE_MS } from '../../live/attention';
import { AUTO_REFRESH_INTERVAL_MS } from '../../live/autoRefresh';
import type { Workbench } from '../../types';
import { overviewResponse, taskId } from '../commands/paletteFixtures';
import {
  COCKPIT_READY,
  defaultApiResponse,
  fixture,
  jsonResponse,
} from '../fixtures';
import { desiredBody, listBody } from '../policy/policyFixtures';
import { deferred } from '../sessionFixtures';

const SECOND = 1000;
const MINUTE = 60 * SECOND;
const WORKBENCH = '/api/workbench';
const OVERVIEW = '/api/v1/board/overview';
const DESIRED = '/api/v1/policy/desired';
const HEALTH = '/api/v1/health';
const STARSHIP = '项目/星舟';

type Reply = () => Response | Promise<Response>;

/** 黑板上两条等你验收的。 */
const board: Parameters<typeof overviewResponse>[0] = [
  {
    id: taskId('A'),
    title: '验收拓扑缩略',
    project: STARSHIP,
    status: 'needs_review',
  },
  {
    id: taskId('B'),
    title: '验收今天的时间线',
    project: STARSHIP,
    status: 'needs_review',
  },
];

/** 策略里一份等你的草案。 */
const drafts = listBody([desiredBody({ id: '01ABCDEFGHJKMNPQRSTVWXYZ02' })]);

const healthy = {
  overall: 'normal',
  evidence_complete: true,
  sections: [
    {
      code: 'disk-quota',
      title: '磁盘配额',
      state: 'normal',
      detail: '占用在预警水位以内',
    },
  ],
};

const fetchMock = vi.fn<typeof fetch>();
/** 四路的应答，每例按需改。 */
let workbench: Reply;
let overview: Reply;
let desired: Reply;
let health: Reply;

beforeEach(() => {
  vi.useFakeTimers({
    shouldAdvanceTime: true,
    toFake: [
      'setTimeout',
      'clearTimeout',
      'setInterval',
      'clearInterval',
      'Date',
    ],
  });
  vi.setSystemTime(new Date(2026, 10, 12, 9, 0, 0));
  workbench = () => jsonResponse(fixture);
  overview = () => jsonResponse(overviewResponse(board));
  desired = () => jsonResponse(drafts);
  health = () => jsonResponse(healthy);
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = String(input);
    if (path.startsWith(WORKBENCH)) return workbench();
    if (path.startsWith(OVERVIEW)) return overview();
    if (path === DESIRED) return desired();
    if (path === HEALTH) return health();
    return defaultApiResponse(input);
  });
  vi.stubGlobal('fetch', fetchMock);
});

afterEach(() => {
  // 先卸载（调度器在假时钟下清掉自己的定时器），再换回真时钟。
  cleanup();
  vi.useRealTimers();
  Reflect.deleteProperty(document, 'visibilityState');
  for (const stray of document.querySelectorAll('dialog[data-stray]'))
    stray.remove();
});

/** 发往某个接口的请求，按先后。 */
const requests = (prefix: string) =>
  fetchMock.mock.calls
    .map(([input]) => String(input))
    .filter((path) => path.startsWith(prefix));

/** 四路各读了几次。顶栏状态胶囊另读一份本机健康（只在挂载时读一次），所以健康那一路从 2 起。 */
const reads = () => ({
  workbench: requests(WORKBENCH).length,
  overview: requests(OVERVIEW).length,
  desired: requests(DESIRED).length,
  health: requests(HEALTH).length,
});

/** 首屏读完之后的读数：四路各一次，外加状态胶囊的那一份健康。 */
const FIRST = { workbench: 1, overview: 1, desired: 1, health: 2 };
/** 再安静重读 n 轮之后。 */
const after = (rounds: number) => ({
  workbench: FIRST.workbench + rounds,
  overview: FIRST.overview + rounds,
  desired: FIRST.desired + rounds,
  health: FIRST.health + rounds,
});

/** 驾驶舱巨号上的数；不可得时是「—」。 */
function hero() {
  return screen
    .getByRole('region', { name: '等你处理' })
    .querySelector('.cockpit-queue__count-value')?.textContent;
}

/** 顶栏「驾驶舱」旁的计数。 */
function badge() {
  return screen
    .getByRole('button', { name: '驾驶舱' })
    .querySelector('.shell-nav-count')?.textContent;
}

/** 「读于 N 秒前」那一处；还没读成过时不存在。 */
const stamp = () => document.querySelector('.live-stamp');

/** 把时钟往前拨。 */
async function elapse(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

/** 切走或切回这个标签页。 */
async function setVisibility(state: 'visible' | 'hidden') {
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => state,
  });
  await act(async () => {
    document.dispatchEvent(new Event('visibilitychange'));
  });
}

/** 挂上整页，等首屏四路都读完（巨号给出数：工作台 1 条 + 黑板 2 条 + 策略 1 份）。 */
async function mount() {
  render(<App />);
  await screen.findByRole('heading', COCKPIT_READY);
  await waitFor(() => expect(hero()).toBe('4'));
  await waitFor(() => expect(reads()).toEqual(FIRST));
}

describe('驾驶舱开着就自己更新', () => {
  it('每 20 秒四路各读一次，快照不带 refresh=true；到点之前不读', async () => {
    await mount();
    expect(stamp()).toHaveTextContent('读于 刚刚');

    await elapse(AUTO_REFRESH_INTERVAL_MS - 5 * SECOND);
    expect(reads()).toEqual(FIRST);

    await elapse(6 * SECOND);
    await waitFor(() => expect(reads()).toEqual(after(1)));
    // 自动刷新不许让服务端把整库重新派生一遍（BUG-035）。
    expect(requests(WORKBENCH)).toEqual([WORKBENCH, WORKBENCH]);

    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    await waitFor(() => expect(reads()).toEqual(after(2)));
    expect(requests(WORKBENCH)).toEqual([WORKBENCH, WORKBENCH, WORKBENCH]);
  });

  it('重读期间不显示读取中、巨号不变成「—」；读成了两处一起换成新数', async () => {
    await mount();
    expect(badge()).toBe('4');

    // 下一轮：工作台多一条待办，黑板多一条等你的；快照那一路先压着不放。
    const held = deferred<Response>();
    workbench = () => held.promise;
    overview = () =>
      jsonResponse(
        overviewResponse([
          ...board,
          {
            id: taskId('C'),
            title: '验收命令面板',
            project: STARSHIP,
            status: 'needs_review',
          },
        ]),
      );
    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    expect(requests(WORKBENCH)).toHaveLength(2);
    // 快照还在路上：页面照旧，后面几路排着没发（同时在途最多一个）。
    expect(hero()).toBe('4');
    expect(badge()).toBe('4');
    expect(screen.queryByText('正在读取工作台…')).not.toBeInTheDocument();
    expect(
      screen.queryByText('正在读取黑板里等你的任务…'),
    ).not.toBeInTheDocument();
    expect(requests(OVERVIEW)).toHaveLength(1);

    const next: Workbench = {
      ...fixture,
      generation: fixture.generation + 1,
      inbox: [
        ...fixture.inbox,
        { ...fixture.inbox[0], id: 'action-2', title: '验收自动刷新' },
      ],
    };
    await act(async () => held.resolve(jsonResponse(next)));
    await waitFor(() => expect(hero()).toBe('6'));
    expect(badge()).toBe('6');
    expect(
      screen.getByRole('button', { name: '验收自动刷新，查看原文' }),
    ).toBeInTheDocument();
    await waitFor(() => expect(reads()).toEqual(after(1)));
  });

  it('没读成：留着上一次的数并写明原因，下一次隔得更久；「现在重读」立刻再读', async () => {
    await mount();
    workbench = () => jsonResponse({ error: '副本不可用' }, 503);
    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    const note = await screen.findByText(/自动刷新没成功：副本不可用/);
    expect(note).toHaveAttribute('role', 'status');
    // 数据一格没动；别的几路照常读完。
    expect(hero()).toBe('4');
    expect(badge()).toBe('4');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    await waitFor(() => expect(reads()).toEqual(after(1)));

    // 连着没读成，间隔加倍：再过 20 秒不读，过 40 秒才读。
    await elapse(AUTO_REFRESH_INTERVAL_MS + 5 * SECOND);
    expect(requests(WORKBENCH)).toHaveLength(2);
    await elapse(AUTO_REFRESH_INTERVAL_MS);
    await waitFor(() => expect(reads()).toEqual(after(2)));
    expect(screen.getByText(/自动刷新没成功：副本不可用/)).toBeInTheDocument();

    workbench = () => jsonResponse(fixture);
    fireEvent.click(screen.getByRole('button', { name: '现在重读' }));
    await waitFor(() => expect(reads()).toEqual(after(3)));
    await waitFor(() =>
      expect(screen.queryByText(/自动刷新没成功/)).not.toBeInTheDocument(),
    );
    expect(stamp()).toHaveTextContent('读于 刚刚');
    // 读成之后恢复 20 秒一读。
    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    await waitFor(() => expect(reads()).toEqual(after(4)));
  });

  it('手动刷新读完重新计时，不紧跟着再读一遍', async () => {
    await mount();
    await elapse(15 * SECOND);
    fireEvent.click(screen.getByRole('button', { name: /^工作台状态/ }));
    fireEvent.click(screen.getByRole('button', { name: '刷新快照' }));
    // 手动刷新带 refresh=true；读完连带重读黑板汇总、策略草案，状态胶囊和驾驶舱各重读一次健康。
    const manual = { workbench: 2, overview: 2, desired: 2, health: 4 };
    await waitFor(() => expect(reads()).toEqual(manual));
    await waitFor(() => expect(hero()).toBe('4'));
    expect(requests(WORKBENCH)).toEqual([
      WORKBENCH,
      `${WORKBENCH}?refresh=true`,
    ]);

    // 离首屏已经过了 20 秒，但离这次刷新才 10 秒：不读。
    await elapse(10 * SECOND);
    expect(requests(WORKBENCH)).toHaveLength(2);
    await elapse(11 * SECOND);
    await waitFor(() =>
      expect(reads()).toEqual({
        workbench: manual.workbench + 1,
        overview: manual.overview + 1,
        desired: manual.desired + 1,
        health: manual.health + 1,
      }),
    );
    expect(requests(WORKBENCH)[2]).toBe(WORKBENCH);
  });

  it('首次读取没读成：过一个间隔自己再试，读成了页面就出来', async () => {
    workbench = () => jsonResponse({ error: '副本不可用' }, 422);
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('副本不可用');
    expect(stamp()).toBeNull();

    workbench = () => jsonResponse(fixture);
    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    await screen.findByRole('heading', COCKPIT_READY);
    await waitFor(() => expect(hero()).toBe('4'));
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(requests(WORKBENCH)).toEqual([WORKBENCH, WORKBENCH]);
  });
});

describe('什么时候不读', () => {
  it('开着原文阅读器不读，关掉之后立刻补读', async () => {
    await mount();
    fireEvent.click(
      screen.getByRole('button', { name: '验收只读工作台，查看原文' }),
    );
    await screen.findByRole('button', { name: '关闭原文' });
    await elapse(3 * MINUTE);
    expect(reads()).toEqual(FIRST);

    fireEvent.click(screen.getByRole('button', { name: '关闭原文' }));
    await elapse(SECOND);
    await waitFor(() => expect(reads()).toEqual(after(1)));
  });

  it('开着命令面板不读，关掉之后立刻补读', async () => {
    await mount();
    fireEvent.keyDown(window, { key: 'k', metaKey: true });
    await screen.findByRole('dialog', { name: '动作面板' });
    // 面板打开时自己另读一份黑板汇总（「跳到黑板任务」用），那一次不是自动刷新发的。
    const opened = { ...FIRST, overview: FIRST.overview + 1 };
    await waitFor(() => expect(reads()).toEqual(opened));
    await elapse(3 * MINUTE);
    expect(reads()).toEqual(opened);

    fireEvent.keyDown(window, { key: 'k', metaKey: true });
    await waitFor(() =>
      expect(
        screen.queryByRole('dialog', { name: '动作面板' }),
      ).not.toBeInTheDocument(),
    );
    await elapse(SECOND);
    await waitFor(() =>
      expect(reads()).toEqual({ ...after(1), overview: opened.overview + 1 }),
    );
  });

  it('别的弹层开着（验证码、会话、最近删除……）也不读，弹层一关就补读', async () => {
    await mount();
    const stray = document.createElement('dialog');
    stray.dataset.stray = 'true';
    stray.setAttribute('open', '');
    document.body.append(stray);
    await elapse(3 * MINUTE);
    expect(reads()).toEqual(FIRST);

    stray.remove();
    await elapse(3 * SECOND);
    await waitFor(() => expect(reads()).toEqual(after(1)));
  });

  it('标签页切走不读；很快切回来不多读，切走久了回来立刻补读', async () => {
    await mount();
    await elapse(5 * SECOND);
    await setVisibility('hidden');
    await elapse(5 * SECOND);
    await setVisibility('visible');
    // 数据才 10 秒：一个请求都不发（ADR-022 的「切回不发请求」对内容读取同样成立）。
    await elapse(5 * SECOND);
    expect(reads()).toEqual(FIRST);

    await setVisibility('hidden');
    await elapse(3 * MINUTE);
    expect(reads()).toEqual(FIRST);
    await setVisibility('visible');
    await elapse(SECOND);
    await waitFor(() => expect(reads()).toEqual(after(1)));
    expect(stamp()).toHaveTextContent('读于 刚刚');
  });

  it('切走时在途的那一轮作废：后面几路不再发，回来重新读一轮', async () => {
    await mount();
    const held = deferred<Response>();
    workbench = () => held.promise;
    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    expect(requests(WORKBENCH)).toHaveLength(2);

    await setVisibility('hidden');
    await act(async () =>
      held.resolve(
        jsonResponse({ ...fixture, generation: fixture.generation + 1 }),
      ),
    );
    await elapse(SECOND);
    expect(reads()).toEqual({ ...FIRST, workbench: 2 });

    workbench = () => jsonResponse(fixture);
    await setVisibility('visible');
    await elapse(SECOND);
    await waitFor(() => expect(reads()).toEqual({ ...after(1), workbench: 3 }));
  });

  it('五分钟没有操作就先停着并写明；一有操作立刻恢复并补读', async () => {
    await mount();
    await elapse(ATTENTION_IDLE_MS + 10 * SECOND);
    expect(
      await screen.findByText('有一会儿没操作，自动刷新先停着'),
    ).toBeInTheDocument();
    const paused = requests(WORKBENCH).length;
    // 停之前照常 20 秒一读（5 分钟里十几轮），停了之后一轮都不读：服务端的闲置登出照常计时。
    expect(paused).toBeGreaterThan(10);
    await elapse(10 * MINUTE);
    expect(requests(WORKBENCH)).toHaveLength(paused);

    fireEvent.pointerDown(document.body);
    await waitFor(() =>
      expect(
        screen.queryByText('有一会儿没操作，自动刷新先停着'),
      ).not.toBeInTheDocument(),
    );
    await elapse(SECOND);
    await waitFor(() => expect(requests(WORKBENCH)).toHaveLength(paused + 1));
    expect(stamp()).toHaveTextContent('读于 刚刚');
  });

  it('别的视图不读；回到驾驶舱立刻补读', async () => {
    await mount();
    fireEvent.click(screen.getByRole('button', { name: '项目' }));
    await elapse(3 * MINUTE);
    expect(reads()).toEqual(FIRST);

    fireEvent.click(screen.getByRole('button', { name: '驾驶舱' }));
    await elapse(SECOND);
    // 驾驶舱重新挂上时自己先读一次健康，这一轮安静重读再读一次。
    await waitFor(() =>
      expect(reads()).toEqual({ ...after(1), health: FIRST.health + 2 }),
    );
  });
});

describe('协作拓扑', () => {
  it('和角标、驾驶舱用同一份黑板汇总，不另读；开着同样每 20 秒读一轮，换到别的标签就不读', async () => {
    await mount();
    fireEvent.click(screen.getByRole('button', { name: '协作' }));
    const topology = await screen.findByRole('tab', { name: '拓扑' });
    expect(topology).toHaveAttribute('aria-selected', 'true');
    // 进协作页不再多读一份汇总（原来各读各的）。
    expect(requests(OVERVIEW)).toHaveLength(1);
    expect(stamp()).toHaveTextContent('读于');

    await elapse(AUTO_REFRESH_INTERVAL_MS + SECOND);
    // 离开了驾驶舱：健康那一路不在这一轮里。
    await waitFor(() =>
      expect(reads()).toEqual({ ...after(1), health: FIRST.health }),
    );

    fireEvent.click(screen.getByRole('tab', { name: '任务与回执' }));
    const settled = reads();
    await elapse(3 * MINUTE);
    expect(requests(WORKBENCH)).toHaveLength(settled.workbench);
    expect(requests(OVERVIEW)).toHaveLength(settled.overview);

    fireEvent.click(screen.getByRole('tab', { name: '拓扑' }));
    await elapse(SECOND);
    await waitFor(() =>
      expect(requests(OVERVIEW)).toHaveLength(settled.overview + 1),
    );
  });

  it('工作台快照读不出来时照样读得到黑板', async () => {
    workbench = () => jsonResponse({ error: '副本不可用' }, 422);
    window.location.hash = '#view=collab';
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('副本不可用');
    expect(
      await screen.findByRole('img', { name: '协作拓扑' }),
    ).toBeInTheDocument();
    expect(screen.getByText(/截至/)).toBeInTheDocument();
    expect(requests(OVERVIEW)).toHaveLength(1);
    // 策略草案那一路要等快照，这时不读。
    expect(requests(DESIRED)).toHaveLength(0);
  });
});
