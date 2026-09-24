/*
 * 驾驶舱与协作页对同一份黑板汇总画出一模一样的「今天的时间线」，驾驶舱拓扑缩略、协作页拓扑、今天泳道
 * 三处对同一执行者叫同一个名字（T-130 C4：执行者标签与泳道规则只留一套）。
 *
 * 做法：把 todayLanes 包一层，记下两页真正调用它时的入参与结果（泳道键、名字、块区间逐一比），
 * 再比两页渲染出来的泳道图；拓扑的名字从两张图的「按执行者列出」清单里读。
 */
import {
  cleanup,
  configure,
  render,
  screen,
  within,
} from '@testing-library/react';
import { createElement } from 'react';
import { describe, expect, it, vi } from 'vitest';
import {
  ATTEMPT,
  INSTANCE,
  POLICY_HASH,
  TASK,
} from '../../../dev/fixtures/ids';
import type { BoardOverview } from '../../boardOverviewModel';
import { executorOf, overviewLabels } from '../../collab/executorGrouping';
import { OWNER_KEY, type TodayLanes } from '../../collab/todayLanes';
import { CollabView } from '../../views/CollabView';
import { CockpitHarness } from '../cockpit/CockpitHarness';
import {
  fixtureData,
  fixtureFetch,
  type FixtureData,
} from '../cockpit/cockpitFixtures';

// 同 CollabView.test.tsx：整页 DOM 很大，jsdom 不加载样式表，角色查询不必逐节点判可见性。
configure({ defaultHidden: true, asyncUtilTimeout: 30_000 });

const calls = vi.hoisted(
  () => [] as { overview: BoardOverview; result: TodayLanes }[],
);
vi.mock('../../collab/todayLanes', async (importOriginal) => {
  const actual =
    await importOriginal<typeof import('../../collab/todayLanes')>();
  return {
    ...actual,
    todayLanes: (...args: Parameters<typeof actual.todayLanes>) => {
      const result = actual.todayLanes(...args);
      calls.push({ overview: args[0], result });
      return result;
    },
  };
});

const OVERVIEW = '/api/v1/board/overview';
const HOUR = 3_600_000;
const STUDIOLO = '项目/Studiolo';
const BRIEF = '项目/Brief 简报站';
const noop = () => {};

function json(value: unknown): Response {
  return new Response(JSON.stringify(value), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

/** 驾驶舱整页（拓扑缩略与时间线格都是真品）；等泳道画出来再返回。 */
async function mountCockpit(data: FixtureData, wire?: unknown) {
  vi.stubGlobal(
    'fetch',
    fixtureFetch(
      data.api,
      wire === undefined ? {} : { [OVERVIEW]: () => json(wire) },
    ),
  );
  const { container } = render(
    createElement(CockpitHarness, {
      workbench: data.workbench,
      refreshing: false,
      onRefresh: noop,
      filters: null,
      actions: data.workbench.inbox,
      projects: data.workbench.projects,
      diagnostics: 0,
      onOpen: noop,
      onEnterProject: noop,
      onViewProjects: noop,
      onShowDiagnostics: noop,
    }),
  );
  await within(screen.getByRole('region', { name: '今天的时间线' })).findByRole(
    'img',
    { name: /^时间泳道/ },
  );
  return container;
}

/** 协作页（拓扑标签）；hash 决定项目筛选与选中条目。 */
async function mountCollab(data: FixtureData, hash: string, wire?: unknown) {
  window.location.hash = hash;
  vi.stubGlobal(
    'fetch',
    fixtureFetch(
      data.api,
      wire === undefined ? {} : { [OVERVIEW]: () => json(wire) },
    ),
  );
  const { container } = render(
    createElement(CollabView, {
      projects: [
        { root: STUDIOLO, name: 'Studiolo' },
        { root: BRIEF, name: 'Brief 简报站' },
      ],
    }),
  );
  await screen.findByRole('img', { name: /^时间泳道/ });
  return container;
}

/** 这一页最后一次调用 todayLanes 的入参与结果（页面画的就是它）。 */
function lastCall() {
  const call = calls.at(-1);
  if (!call) throw new Error('页面没有调用 todayLanes');
  return call;
}

/** 泳道的键、名字、块区间与范围：两页要逐一相等的部分。 */
const shape = (today: TodayLanes) => ({
  range: today.range,
  hidden: today.hidden,
  events: today.events,
  lanes: today.lanes.map(({ key, name, blocks }) => ({
    key,
    name,
    blocks: blocks.map(({ start, end, tone }) => ({ start, end, tone })),
  })),
});

/** 页面上唯一一张泳道图画出来的样子：读屏摘要、泳道名、每条轨道上的块（类名、x、宽）、刻度。 */
function lanesPicture(root: HTMLElement) {
  const figures = root.querySelectorAll('.ui-lanes');
  expect(figures).toHaveLength(1);
  const figure = figures[0];
  const texts = (selector: string) =>
    [...figure.querySelectorAll(selector)].map((node) => node.textContent);
  return {
    label: figure.getAttribute('aria-label'),
    names: texts('.ui-lanes__name'),
    tracks: [...figure.querySelectorAll('.ui-lanes__track')].map((track) =>
      [...track.querySelectorAll('rect')].map((rect) =>
        ['class', 'x', 'width']
          .map((name) => rect.getAttribute(name))
          .join(' '),
      ),
    ),
    ticks: texts('.ui-lanes__ticks > span'),
  };
}

/** 拓扑「按执行者列出」清单里的执行者名：执行者键 → 名字（任务按标题对回汇总条目）。 */
function topologyNames(
  root: HTMLElement,
  overview: BoardOverview,
): Map<string, string> {
  const byTitle = new Map(overview.items.map((item) => [item.title, item]));
  const names = new Map<string, string>();
  for (const list of root.querySelectorAll('ul.topology__entries')) {
    const name = (list.getAttribute('aria-label') ?? '').replace(
      / 的任务$/,
      '',
    );
    for (const entry of list.querySelectorAll('.topology__entry-select')) {
      const item = byTitle.get(entry.textContent ?? '');
      expect(
        item,
        `清单里的任务「${entry.textContent}」不在汇总里`,
      ).toBeDefined();
      if (item) names.set(executorOf(item).key, name);
    }
  }
  return names;
}

describe(
  '驾驶舱与协作页：同一份汇总，同一套泳道与执行者名',
  { timeout: 180_000 },
  () => {
    it('假数据：两页走同一个 todayLanes、入参相同，泳道键、名字、块区间逐一相等，画出来也一样；三处拓扑与泳道同名', async () => {
      const data = fixtureData();
      calls.length = 0;
      const cockpit = await mountCockpit(data);
      const fromCockpit = lastCall();
      const cockpitPicture = lanesPicture(cockpit);
      const mini = topologyNames(cockpit, fromCockpit.overview);
      cleanup();

      calls.length = 0;
      const collab = await mountCollab(data, '#view=collab');
      const fromCollab = lastCall();
      const collabPicture = lanesPicture(collab);
      const full = topologyNames(collab, fromCollab.overview);

      // 同一个函数、同一份汇总（两页各读一次接口，内容相同），结果逐一相等。
      expect(fromCollab.overview).toEqual(fromCockpit.overview);
      expect(shape(fromCollab.result)).toEqual(shape(fromCockpit.result));
      expect(fromCockpit.result.lanes.map((lane) => lane.key)).toEqual([
        `instance:${INSTANCE.R3}`,
        `instance:${INSTANCE.R4}`,
        `instance:${INSTANCE.R1}`,
        `instance:${INSTANCE.R2}`,
        OWNER_KEY,
      ]);
      // 画出来也一样：泳道名、每个块的位置与宽度、刻度、读屏摘要。
      expect(collabPicture).toEqual(cockpitPicture);
      expect(cockpitPicture.names.slice(0, 5)).toEqual([
        'Claude · 01M2Y',
        'Claude · 01M31',
        'Claude · 01M34',
        'Codex · 01M2J',
        '你',
      ]);

      // 三处同名：驾驶舱缩略画了 6 条（五位执行者），协作页画全部 7 条。
      const labels = overviewLabels(fromCockpit.overview);
      expect([...mini.keys()].sort()).toEqual(
        [
          `instance:${INSTANCE.R1}`,
          `instance:${INSTANCE.R2}`,
          `instance:${INSTANCE.R3}`,
          `instance:${INSTANCE.R4}`,
          `instance:${INSTANCE.R5}`,
        ].sort(),
      );
      for (const [key, name] of mini) {
        expect(full.get(key)).toBe(name);
        expect(labels.get(key)).toBe(name);
      }
      for (const lane of fromCockpit.result.lanes) {
        if (lane.key === OWNER_KEY) continue;
        expect(mini.get(lane.key)).toBe(lane.name);
        expect(full.get(lane.key)).toBe(lane.name);
      }
    });

    it('缩略图与按项目筛过的协作页只画汇总的一部分：执行者名仍按整份汇总取，三处都是「Claude · 01M34」', async () => {
      /*
       * 两个 Claude 根实例前 4 位都是 01M3：R1 在跑 Studiolo 的任务，R4 今天在简报站干完一件。
       * 缩略图只画未结束的那条、筛到 Studiolo 的协作页也只剩它——若各自按画出来的条目起名，
       * 就只剩一个根、前 4 位够用，名字成了「Claude · 01M3」，与泳道的「Claude · 01M34」对不上。
       */
      const data = fixtureData();
      const now = Date.parse(data.overview.generatedAt);
      const today = new Date(now);
      const clock = (hour: number) =>
        new Date(
          today.getFullYear(),
          today.getMonth(),
          today.getDate(),
        ).getTime() + Math.round(hour * HOUR);
      const iso = (ms: number) => new Date(ms).toISOString();
      const event = (
        hour: number,
        task: string,
        project: string,
        action: string,
        phase: 'intent' | 'outcome',
        attempt: string | null,
      ) => ({
        at: iso(clock(hour)),
        project,
        task_id: task,
        action,
        phase,
        attempt_id: attempt,
      });
      const node = (id: string) => ({ id, agent_type: 'claude' });
      const wire = {
        generated_at: iso(now),
        window_from: iso(now - 24 * HOUR),
        window_to: iso(now),
        policy_hash: POLICY_HASH.v3,
        items: [
          {
            id: TASK.T106,
            title: '简报站验收（今天已结束）',
            project: BRIEF,
            status: 'succeeded',
            parent: null,
            adapter: 'claude',
            holder: INSTANCE.C4a,
            holder_lineage: [node(INSTANCE.R4), node(INSTANCE.C4a)],
            paths: [],
            attempt: {
              phase: 'completed',
              worker: INSTANCE.C4a,
              active: false,
              lease_until_ms: clock(11.2),
            },
            progress: { tools: 3, edits: 1, last_progress_ms: clock(11) },
            receipt_status: 'completed',
          },
          {
            id: TASK.T130,
            title: '重设计（在跑）',
            project: STUDIOLO,
            status: 'running',
            parent: null,
            adapter: 'claude',
            holder: INSTANCE.C1,
            holder_lineage: [node(INSTANCE.R1), node(INSTANCE.C1)],
            paths: ['web'],
            attempt: {
              phase: 'running',
              worker: INSTANCE.C1,
              active: true,
              lease_until_ms: now + HOUR,
            },
            progress: {
              tools: 9,
              edits: 2,
              last_progress_ms: now - 10 * 60_000,
            },
            receipt_status: null,
          },
        ],
        items_truncated: false,
        events: [
          event(9, TASK.T106, BRIEF, 'task.claim', 'outcome', ATTEMPT.T106),
          event(
            10,
            TASK.T106,
            BRIEF,
            'task.heartbeat',
            'outcome',
            ATTEMPT.T106,
          ),
          event(
            11,
            TASK.T106,
            BRIEF,
            'task.runner_candidates',
            'outcome',
            ATTEMPT.T106,
          ),
          event(11.5, TASK.T106, BRIEF, 'execution.review', 'intent', null),
          event(
            11.5 + 1 / 3600,
            TASK.T106,
            BRIEF,
            'execution.review',
            'outcome',
            null,
          ),
          event(
            19.5,
            TASK.T130,
            STUDIOLO,
            'task.claim',
            'outcome',
            ATTEMPT.T130,
          ),
          event(
            20,
            TASK.T130,
            STUDIOLO,
            'task.heartbeat',
            'outcome',
            ATTEMPT.T130,
          ),
        ],
        events_truncated: false,
      };
      const key = `instance:${INSTANCE.R1}`;
      const expected = 'Claude · 01M34';

      // 驾驶舱：缩略图只画在跑的一条，泳道有两位执行者。
      calls.length = 0;
      const cockpit = await mountCockpit(data, wire);
      const fromCockpit = lastCall();
      expect(fromCockpit.overview.items).toHaveLength(2);
      const mini = topologyNames(cockpit, fromCockpit.overview);
      expect([...mini]).toEqual([[key, expected]]);
      expect(
        fromCockpit.result.lanes.map(({ key: laneKey, name }) => [
          laneKey,
          name,
        ]),
      ).toEqual([
        [`instance:${INSTANCE.R4}`, 'Claude · 01M31'],
        [key, expected],
        [OWNER_KEY, '你'],
      ]);
      const cockpitPicture = lanesPicture(cockpit);
      cleanup();

      // 协作页「全部项目」：与驾驶舱的泳道逐一相等，拓扑同名。
      calls.length = 0;
      const collab = await mountCollab(data, '#view=collab', wire);
      expect(shape(lastCall().result)).toEqual(shape(fromCockpit.result));
      expect(lanesPicture(collab)).toEqual(cockpitPicture);
      expect([...topologyNames(collab, fromCockpit.overview)]).toEqual([
        [key, expected],
      ]);
      cleanup();

      // 协作页筛到 Studiolo 并选中这条：泳道只画这个项目的事件，拓扑、泳道、详情仍叫同一个名字。
      calls.length = 0;
      const scoped = await mountCollab(
        data,
        `#view=collab&project=${encodeURIComponent(STUDIOLO)}&item=${TASK.T130}`,
        wire,
      );
      const fromScoped = lastCall();
      expect(fromScoped.overview.items.map((item) => item.id)).toEqual([
        TASK.T130,
      ]);
      expect(
        fromScoped.result.lanes.map(({ key: laneKey, name }) => [
          laneKey,
          name,
        ]),
      ).toEqual([
        [key, expected],
        [OWNER_KEY, '你'],
      ]);
      expect([...topologyNames(scoped, fromCockpit.overview)]).toEqual([
        [key, expected],
      ]);
      expect(
        screen.getByRole('complementary', { name: '任务详情' }),
      ).toHaveTextContent(`${expected} / ${INSTANCE.C1.slice(0, 8)}`);
    });
  },
);
