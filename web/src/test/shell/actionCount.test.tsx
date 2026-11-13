/*
 * 顶栏「驾驶舱 N」角标与驾驶舱巨号（T-162、T-195）：同一个 pendingCount、同一份工作台快照、
 * 同一次黑板汇总读取、同一次策略草案读取，两处任何时刻都相等。两份都由 App 读，驾驶舱经 props 拿同一份：
 * 驾驶舱页面上各只请求一次，离开驾驶舱（或根本没进过驾驶舱）时角标照样含黑板里等你的任务和策略里等你的提案；
 * 刷新结束两处一起重读、一起更新。
 */
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../../App';
import type { Workbench } from '../../types';
import { overviewResponse, taskId } from '../commands/paletteFixtures';
import { defaultApiResponse, fixture, jsonResponse } from '../fixtures';
import { desiredBody, listBody } from '../policy/policyFixtures';
import { deferred } from '../sessionFixtures';

const OVERVIEW = '/api/v1/board/overview';
const DESIRED = '/api/v1/policy/desired';
const STARSHIP = '项目/星舟';
const isOverview = (input: unknown) => String(input).startsWith(OVERVIEW);
const isDesired = (input: unknown) => String(input) === DESIRED;

/** 黑板上四条：两条待验收（等你）、一条排队、一条已结束；等你的只有前两条。 */
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
  {
    id: taskId('C'),
    title: '排队中的盘点',
    project: STARSHIP,
    status: 'queued',
  },
  {
    id: taskId('D'),
    title: '已经收尾的盘点',
    project: STARSHIP,
    status: 'succeeded',
  },
];

/** 策略里两份等你的：一份 AI 提案（待复核）、一份你自己交的草案（已校验）；外加一份已发布的，不算。 */
const drafts = listBody([
  desiredBody({
    id: '01ABCDEFGHJKMNPQRSTVWXYZ01',
    status: 'pending',
    submitter_kind: 'agent',
    agent_type: 'claude',
    agent_instance: '01ABCDEFGHJKMNPQRSTVWXYZ99',
  }),
  desiredBody({ id: '01ABCDEFGHJKMNPQRSTVWXYZ02', subject: 'codex' }),
  desiredBody({
    id: '01ABCDEFGHJKMNPQRSTVWXYZ03',
    subject: 'consumer',
    status: 'activated',
    activated_version: 3,
  }),
]);

const fetchMock = vi.fn<typeof fetch>();
/** 工作台快照、黑板汇总与策略草案的应答，每例按需改；策略缺省是「本入口没装策略平面」（404）。 */
let snapshot: Workbench;
let overview: () => Response | Promise<Response>;
let desired: (() => Response | Promise<Response>) | null;

beforeEach(() => {
  snapshot = fixture;
  overview = () => jsonResponse(overviewResponse(board));
  desired = null;
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) =>
    isOverview(input)
      ? overview()
      : desired && isDesired(input)
        ? desired()
        : defaultApiResponse(input, snapshot),
  );
  vi.stubGlobal('fetch', fetchMock);
});

/** 顶栏「驾驶舱」旁的计数（对读屏隐藏的等宽小字）；不可得时是「—」。 */
function badge() {
  return screen
    .getByRole('button', { name: '驾驶舱' })
    .querySelector('.shell-nav-count')?.textContent;
}

/** 驾驶舱巨号上的数；不可得时是「—」。 */
function hero() {
  return screen
    .getByRole('region', { name: '等你处理' })
    .querySelector('.cockpit-queue__count-value')?.textContent;
}

/** 同一刻读两处。 */
const both = () => ({ badge: badge(), hero: hero() });

/** 驾驶舱右栏「谁在干活」；工作台快照到手前驾驶舱还没渲染，所以要等它出现。 */
const crew = () => screen.findByRole('complementary', { name: '谁在干活' });

/** 黑板汇总被请求了几次（全站计，含驾驶舱与角标）。 */
function overviewRequests() {
  return fetchMock.mock.calls.filter(([input]) => isOverview(input)).length;
}

/** 策略草案列表被请求了几次（全站计；策略页自己的整页读取不在驾驶舱和别的视图里发生）。 */
function desiredRequests() {
  return fetchMock.mock.calls.filter(([input]) => isDesired(input)).length;
}

describe('四种黑板状态下，顶栏角标与驾驶舱巨号相等', () => {
  it('汇总读取中：巨号不给数，角标也是「—」', async () => {
    overview = () => new Promise<Response>(() => {});
    render(<App />);
    expect(
      await screen.findByText('正在读取黑板里等你的任务…'),
    ).toBeInTheDocument();
    expect(both()).toEqual({ badge: '—', hero: '—' });
  });

  it('读完且有等你的任务：两处都是工作台待办加等你的黑板任务', async () => {
    render(<App />);
    await waitFor(() => expect(hero()).toBe('3'));
    expect(both()).toEqual({ badge: '3', hero: '3' });
  });

  it('黑板未启用（404）：两处都只数工作台待办', async () => {
    overview = () => new Response(null, { status: 404 });
    render(<App />);
    expect(
      await within(await crew()).findByText('黑板未启用'),
    ).toBeInTheDocument();
    await waitFor(() => expect(hero()).toBe('1'));
    expect(both()).toEqual({ badge: '1', hero: '1' });
  });

  it('读取失败：巨号显示「—」，角标也显示「—」', async () => {
    overview = () => jsonResponse({ error: '黑板许可已满。' }, 429);
    render(<App />);
    expect(await within(await crew()).findByRole('alert')).toHaveTextContent(
      '黑板许可已满。',
    );
    expect(both()).toEqual({ badge: '—', hero: '—' });
  });
});

describe('策略里等你的提案和草案也算进去，两处仍相等（T-195）', () => {
  it('读到了：两处都是工作台待办＋等你的黑板任务＋等你的提案和草案；已发布的不算', async () => {
    desired = () => jsonResponse(drafts);
    render(<App />);
    await waitFor(() => expect(hero()).toBe('5'));
    expect(both()).toEqual({ badge: '5', hero: '5' });
    expect(desiredRequests()).toBe(1);
  });

  it('读取中：两处都是「—」，不拿另外两路冒充总数', async () => {
    desired = () => new Promise<Response>(() => {});
    render(<App />);
    expect(
      await screen.findByText('正在读取策略里等你的提案和草案…'),
    ).toBeInTheDocument();
    // 黑板那一路已经读完，队列里先列出来了，但总数仍不给。
    expect(await screen.findByText('验收拓扑缩略')).toBeInTheDocument();
    expect(both()).toEqual({ badge: '—', hero: '—' });
  });

  it('读取失败：两处都是「—」；就地重试成功后两处一起给数，黑板汇总不重读', async () => {
    let failing = true;
    desired = () =>
      failing
        ? jsonResponse({ error: '策略服务暂时不可用。' }, 503)
        : jsonResponse(drafts);
    const user = userEvent.setup();
    render(<App />);
    const retry = await screen.findByRole('button', { name: '重试策略草案' });
    expect(both()).toEqual({ badge: '—', hero: '—' });
    failing = false;
    await user.click(retry);
    await waitFor(() => expect(hero()).toBe('5'));
    expect(both()).toEqual({ badge: '5', hero: '5' });
    expect(desiredRequests()).toBe(2);
    expect(overviewRequests()).toBe(1);
  });

  it('离开驾驶舱、或根本没进过驾驶舱，角标照样含策略里等你的；策略页自己读自己的，不借这一份', async () => {
    desired = () => jsonResponse(drafts);
    window.location.hash = '#view=projects';
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(badge()).toBe('5'));
    expect(desiredRequests()).toBe(1);
    await user.click(screen.getByRole('button', { name: '文档' }));
    expect(badge()).toBe('5');
    expect(desiredRequests()).toBe(1);
  });
});

describe('黑板汇总由 App 读一次，角标与驾驶舱共用', () => {
  it('驾驶舱页面上只请求一次；离开再回来，驾驶舱也不另读', async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(hero()).toBe('3'));
    expect(badge()).toBe('3');
    expect(overviewRequests()).toBe(1);
    expect(desiredRequests()).toBe(1);
    await user.click(screen.getByRole('button', { name: '项目' }));
    await user.click(screen.getByRole('button', { name: '驾驶舱' }));
    // 回到驾驶舱即给数：用的还是 App 那两份，不经过读取中。
    expect(both()).toEqual({ badge: '3', hero: '3' });
    expect(overviewRequests()).toBe(1);
    expect(desiredRequests()).toBe(1);
  });

  it('切到别的视图，角标仍含黑板里等你的任务', async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(hero()).toBe('3'));
    for (const view of ['项目', '文档', '协作', '策略']) {
      await user.click(screen.getByRole('button', { name: view }));
      expect(
        screen.queryByRole('region', { name: '等你处理' }),
      ).not.toBeInTheDocument();
      expect(badge()).toBe('3');
    }
  });

  it('直接从别的视图打开、没进过驾驶舱：角标同样含黑板里等你的任务', async () => {
    window.location.hash = '#view=projects';
    render(<App />);
    await waitFor(() => expect(badge()).toBe('3'));
    expect(
      screen.queryByRole('region', { name: '等你处理' }),
    ).not.toBeInTheDocument();
    expect(overviewRequests()).toBe(1);
  });

  it('工作台快照读不出来时不读黑板汇总、角标是「—」；重试拿到快照后只读一次', async () => {
    fetchMock.mockImplementationOnce(async () =>
      jsonResponse({ error: '副本不可用' }, 422),
    );
    const user = userEvent.setup();
    render(<App />);
    expect(await screen.findByRole('alert')).toHaveTextContent('副本不可用');
    expect(badge()).toBe('—');
    expect(overviewRequests()).toBe(0);
    expect(desiredRequests()).toBe(0);
    await user.click(screen.getByRole('button', { name: '重试' }));
    await waitFor(() => expect(hero()).toBe('3'));
    expect(both()).toEqual({ badge: '3', hero: '3' });
    // 快照到手与这一轮读取结束是同一次提交：首读跟着快照发，不因「刷新结束」再读一遍。
    expect(overviewRequests()).toBe(1);
    expect(desiredRequests()).toBe(1);
  });
});

describe('刷新', () => {
  it('刷新后两处一起更新：刷新结束即重读汇总，重读中两处都是「—」，读完都是新总数', async () => {
    const user = userEvent.setup();
    render(<App />);
    await waitFor(() => expect(hero()).toBe('3'));
    expect(badge()).toBe('3');

    // 刷新后：工作台多一条待办，黑板多一条等你的任务。
    snapshot = {
      ...fixture,
      generation: fixture.generation + 1,
      inbox: [
        ...fixture.inbox,
        { ...fixture.inbox[0], id: 'action-2', title: '验收驾驶舱角标' },
      ],
    };
    const next = deferred<Response>();
    overview = () => next.promise;
    await user.click(screen.getByRole('button', { name: /^工作台状态/ }));
    await user.click(screen.getByRole('button', { name: '刷新快照' }));

    // 新快照已到手、汇总正在重读：两处都不给数。
    expect(
      await screen.findByText('正在读取黑板里等你的任务…'),
    ).toBeInTheDocument();
    expect(both()).toEqual({ badge: '—', hero: '—' });
    expect(overviewRequests()).toBe(2);
    expect(desiredRequests()).toBe(2);

    await act(async () =>
      next.resolve(
        jsonResponse(
          overviewResponse([
            ...board,
            {
              id: taskId('E'),
              title: '验收命令面板',
              project: STARSHIP,
              status: 'needs_review',
            },
          ]),
        ),
      ),
    );
    await waitFor(() => expect(hero()).toBe('5'));
    expect(both()).toEqual({ badge: '5', hero: '5' });
    expect(overviewRequests()).toBe(2);
  });
});
