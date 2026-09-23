import { afterEach, describe, expect, it } from 'vitest';
import { boardOverviewDisabled, fetchBoardOverview } from '../boardOverviewApi';
import {
  BOARD_OVERVIEW_INVALID,
  parseBoardOverview,
} from '../boardOverviewModel';
import { ApiError, installSessionReader } from '../sessionHttp';

type Json = Record<string, unknown>;
type Sample = Json & { items: Json[]; events: Json[] };

const hash = 'a'.repeat(64);
const id = (n: number) => String(n).padStart(26, '0');
const attemptId = (n: number) => `A-${id(n)}`;
const from = '2026-09-21T19:31:00.123456789Z';
const to = '2026-09-22T19:31:00.123456789Z';
const studiolo = '项目/Studiolo';
const other = '项目/Other';

function draft(n = 3, project = other): Json {
  return {
    id: id(n),
    title: id(n),
    project,
    status: 'draft',
    parent: null,
    adapter: 'codex',
    holder: null,
    holder_lineage: [],
    paths: [`${project}/web`],
    attempt: null,
    progress: null,
    receipt_status: null,
  };
}
function done(): Json {
  return {
    id: id(1),
    title: '已完成的回执',
    project: studiolo,
    status: 'succeeded',
    parent: null,
    adapter: 'consumer',
    holder: 'instance-parent',
    holder_lineage: [{ id: 'instance-parent', agent_type: 'claude' }],
    paths: [`${studiolo}/docs`],
    attempt: {
      phase: 'completed',
      worker: 'instance-parent',
      active: false,
      lease_until_ms: 1_790_105_520_000,
    },
    progress: { tools: 4, edits: 1, last_progress_ms: 1_790_105_400_000 },
    receipt_status: 'completed',
  };
}
function running(): Json {
  return {
    id: id(2),
    title: '前端全面重设计',
    project: studiolo,
    status: 'running',
    parent: null,
    adapter: 'claude',
    holder: 'instance-child',
    holder_lineage: [
      { id: 'instance-parent', agent_type: 'claude' },
      { id: 'instance-child', agent_type: 'claude' },
    ],
    paths: [`${studiolo}/web`, `${studiolo}/docs/design`],
    attempt: {
      phase: 'running',
      worker: 'instance-child',
      active: true,
      lease_until_ms: 1_790_105_580_000,
    },
    progress: { tools: 12, edits: 3, last_progress_ms: 1_790_105_460_000 },
    receipt_status: null,
  };
}
function event(task: string, at: string, extra: Json = {}): Json {
  return {
    at,
    project: studiolo,
    task_id: task,
    action: 'board.request',
    phase: 'outcome',
    attempt_id: attemptId(9),
    ...extra,
  };
}
function sample(): Sample {
  return {
    generated_at: to,
    window_from: from,
    window_to: to,
    policy_hash: hash,
    items: [draft(), done(), running()],
    items_truncated: false,
    events: [
      event(id(1), '2026-09-22T08:00:00Z', { action: 'handoff.returned' }),
      event(id(2), '2026-09-22T08:00:00.5Z', {
        phase: 'intent',
        attempt_id: null,
      }),
      event(id(3), '2026-09-22T09:00:00Z', {
        project: other,
        action: 'task.policy_pinned',
        attempt_id: null,
      }),
    ],
    events_truncated: false,
  };
}
function variant(mutate: (value: Sample) => void): Sample {
  const value = sample();
  mutate(value);
  return value;
}
function rejects(mutate: (value: Sample) => void) {
  expect(() => parseBoardOverview(variant(mutate))).toThrow(
    BOARD_OVERVIEW_INVALID,
  );
}
const at = (value: Sample, index: number, field: string) =>
  value.items[index][field] as Json;

describe('parseBoardOverview', () => {
  it('decodes the closed wire shape into camelCase', () => {
    expect(parseBoardOverview(sample(), 24)).toEqual({
      generatedAt: to,
      windowFrom: from,
      windowTo: to,
      policyHash: hash,
      items: [
        {
          id: id(3),
          title: id(3),
          project: other,
          status: 'draft',
          parent: null,
          adapter: 'codex',
          holder: null,
          holderLineage: [],
          paths: [`${other}/web`],
          attempt: null,
          progress: null,
          receiptStatus: null,
        },
        {
          id: id(1),
          title: '已完成的回执',
          project: studiolo,
          status: 'succeeded',
          parent: null,
          adapter: 'consumer',
          holder: 'instance-parent',
          holderLineage: [{ id: 'instance-parent', agentType: 'claude' }],
          paths: [`${studiolo}/docs`],
          attempt: {
            phase: 'completed',
            worker: 'instance-parent',
            active: false,
            leaseUntilMs: 1_790_105_520_000,
          },
          progress: { tools: 4, edits: 1, lastProgressMs: 1_790_105_400_000 },
          receiptStatus: 'completed',
        },
        {
          id: id(2),
          title: '前端全面重设计',
          project: studiolo,
          status: 'running',
          parent: null,
          adapter: 'claude',
          holder: 'instance-child',
          holderLineage: [
            { id: 'instance-parent', agentType: 'claude' },
            { id: 'instance-child', agentType: 'claude' },
          ],
          paths: [`${studiolo}/web`, `${studiolo}/docs/design`],
          attempt: {
            phase: 'running',
            worker: 'instance-child',
            active: true,
            leaseUntilMs: 1_790_105_580_000,
          },
          progress: {
            tools: 12,
            edits: 3,
            lastProgressMs: 1_790_105_460_000,
          },
          receiptStatus: null,
        },
      ],
      itemsTruncated: false,
      events: [
        {
          at: '2026-09-22T08:00:00Z',
          project: studiolo,
          taskId: id(1),
          action: 'handoff.returned',
          phase: 'outcome',
          attemptId: attemptId(9),
        },
        {
          at: '2026-09-22T08:00:00.5Z',
          project: studiolo,
          taskId: id(2),
          action: 'board.request',
          phase: 'intent',
          attemptId: null,
        },
        {
          at: '2026-09-22T09:00:00Z',
          project: other,
          taskId: id(3),
          action: 'task.policy_pinned',
          phase: 'outcome',
          attemptId: null,
        },
      ],
      eventsTruncated: false,
    });
  });

  it('throws the fixed message for anything that is not an overview', () => {
    for (const value of [null, [], 'overview', 1, {}, { ...sample(), 0: 1 }])
      expect(() => parseBoardOverview(value)).toThrow(
        '黑板汇总格式无效，实时状态不可确认。',
      );
  });

  it.each<[string, (value: Sample) => void]>([
    ['top level', (v) => void (v.extra = 1)],
    ['item', (v) => void (v.items[1].extra = 1)],
    [
      'holder lineage node',
      (v) => void ((at(v, 2, 'holder_lineage') as unknown as Json[])[0].x = 1),
    ],
    ['attempt', (v) => void (at(v, 2, 'attempt').extra = 1)],
    ['progress', (v) => void (at(v, 2, 'progress').extra = 1)],
    ['event', (v) => void (v.events[0].extra = 1)],
  ])('rejects one extra field on the %s', (_, mutate) => rejects(mutate));

  it.each<[string, (value: Sample) => void]>([
    ['top level', (v) => void delete v.events_truncated],
    ['item', (v) => void delete v.items[1].receipt_status],
    [
      'holder lineage node',
      (v) =>
        void delete (at(v, 2, 'holder_lineage') as unknown as Json[])[1]
          .agent_type,
    ],
    ['attempt', (v) => void delete at(v, 2, 'attempt').active],
    ['progress', (v) => void delete at(v, 2, 'progress').last_progress_ms],
    ['event', (v) => void delete v.events[0].attempt_id],
  ])('rejects one missing field on the %s', (_, mutate) => rejects(mutate));

  it.each<[string, (value: Sample) => void]>([
    ['task status', (v) => void (v.items[2].status = 'paused')],
    ['adapter', (v) => void (v.items[2].adapter = 'gpt')],
    ['attempt phase', (v) => void (at(v, 2, 'attempt').phase = 'handed_off')],
    ['receipt status', (v) => void (v.items[1].receipt_status = 'done')],
    ['event phase', (v) => void (v.events[0].phase = 'commit')],
    ['items flag type', (v) => void (v.items_truncated = 'false')],
    ['events flag type', (v) => void (v.events_truncated = null)],
    ['active flag type', (v) => void (at(v, 2, 'attempt').active = 1)],
  ])('rejects an out-of-set %s', (_, mutate) => rejects(mutate));

  it.each<[string, (value: Sample) => void]>([
    [
      'control character in the holder chain',
      (v) => {
        const holder = 'instance\u0007child';
        v.items[2].holder = holder;
        at(v, 2, 'attempt').worker = holder;
        (at(v, 2, 'holder_lineage') as unknown as Json[])[1].id = holder;
      },
    ],
    ['C1 control in an action', (v) => void (v.events[0].action = 'a\u0085')],
    ['empty action', (v) => void (v.events[0].action = ' ')],
    ['lowercase task id', (v) => void (v.items[0].id = 'a'.repeat(26))],
    ['short task id', (v) => void (v.events[0].task_id = id(1).slice(1))],
    ['attempt id prefix', (v) => void (v.events[0].attempt_id = id(9))],
    ['dot-dot path', (v) => void (v.items[2].paths = [`${studiolo}/../x`])],
    ['backslash path', (v) => void (v.items[2].paths = [`${studiolo}\\x`])],
    ['decomposed path', (v) => void (v.items[2].paths = [`${studiolo}/café`])],
    ['trailing slash project', (v) => void (v.items[0].project = `${other}/`)],
    ['short policy hash', (v) => void (v.policy_hash = 'a'.repeat(63))],
    ['negative lease', (v) => void (at(v, 2, 'attempt').lease_until_ms = -1)],
    ['fractional counter', (v) => void (at(v, 2, 'progress').tools = 1.5)],
    ['string counter', (v) => void (at(v, 2, 'progress').edits = '3')],
    [
      'oversized holder',
      (v) => {
        const holder = 'i'.repeat(257);
        v.items[2].holder = holder;
        at(v, 2, 'attempt').worker = holder;
        (at(v, 2, 'holder_lineage') as unknown as Json[])[1].id = holder;
      },
    ],
  ])('rejects %s', (_, mutate) => rejects(mutate));

  it('treats titles as bounded free text', () => {
    const titled = (title: string) =>
      variant((v) => void (v.items[2].title = title));
    expect(parseBoardOverview(titled('第一行\n第二行')).items[2].title).toBe(
      '第一行\n第二行',
    );
    expect(
      parseBoardOverview(titled('标'.repeat(65_536))).items[2].title,
    ).toHaveLength(65_536);
    for (const title of ['', '   ', '标'.repeat(65_537)])
      expect(() => parseBoardOverview(titled(title))).toThrow(
        BOARD_OVERVIEW_INVALID,
      );
  });

  it.each<[string, (value: Sample) => void]>([
    [
      'a non-UTC window end',
      (v) => {
        v.window_to = '2026-09-23T03:31:00.123456789+08:00';
        v.generated_at = v.window_to;
      },
    ],
    [
      'a generation time other than the window end',
      (v) => void (v.generated_at = '2026-09-22T19:31:00.124Z'),
    ],
    [
      'a rolled-over calendar date',
      (v) => void (v.events[2].at = '2026-02-30T09:00:00Z'),
    ],
    [
      'a 24:00 clock time',
      (v) => void (v.events[2].at = '2026-09-21T24:00:00Z'),
    ],
    [
      'a window that is not whole hours',
      (v) => void (v.window_from = '2026-09-21T20:01:00.123456789Z'),
    ],
    ['an empty window', (v) => void (v.window_from = to)],
    [
      'a window beyond one week',
      (v) => void (v.window_from = '2026-09-15T18:31:00.123456789Z'),
    ],
    [
      'an event before the window',
      (v) => void (v.events[0].at = '2026-09-21T19:31:00.122Z'),
    ],
    [
      'an event after the window',
      (v) => void (v.events[2].at = '2026-09-22T19:31:00.124Z'),
    ],
    ['events out of order', (v) => void v.events.reverse()],
  ])('rejects %s', (_, mutate) => rejects(mutate));

  it('accepts events on both window edges and checks the requested span', () => {
    const edges = variant((v) => {
      v.events[0].at = from;
      v.events[2].at = to;
    });
    expect(parseBoardOverview(edges).events).toHaveLength(3);
    expect(() => parseBoardOverview(sample(), 1)).toThrow(
      BOARD_OVERVIEW_INVALID,
    );
    const week = variant((v) => {
      v.window_from = '2026-09-15T19:31:00.123456789Z';
    });
    expect(parseBoardOverview(week, 168).windowFrom).toBe(
      '2026-09-15T19:31:00.123456789Z',
    );
  });

  it.each<[string, (value: Sample) => void]>([
    [
      'a worker other than the holder',
      (v) => void (at(v, 2, 'attempt').worker = 'instance-parent'),
    ],
    [
      'a lineage that does not end at the holder',
      (v) => void (at(v, 2, 'holder_lineage') as unknown as Json[]).reverse(),
    ],
    [
      'a lineage without a holder',
      (v) =>
        void (v.items[0].holder_lineage = [{ id: 'x', agent_type: 'claude' }]),
    ],
    [
      'a holder without a lineage',
      (v) => void (v.items[2].holder_lineage = []),
    ],
    [
      'mixed agent types in one lineage',
      (v) =>
        void ((at(v, 2, 'holder_lineage') as unknown as Json[])[0].agent_type =
          'codex'),
    ],
    [
      'a repeated instance in one lineage',
      (v) =>
        void (v.items[2].holder_lineage = [
          { id: 'instance-child', agent_type: 'claude' },
          { id: 'instance-child', agent_type: 'claude' },
        ]),
    ],
    [
      'a lineage deeper than four',
      (v) =>
        void (v.items[2].holder_lineage = [
          'a',
          'b',
          'c',
          'd',
          'instance-child',
        ].map((name) => ({ id: name, agent_type: 'claude' }))),
    ],
    ['a holder without an attempt', (v) => void (v.items[2].attempt = null)],
    [
      'progress without an attempt',
      (v) =>
        void (v.items[0].progress = {
          tools: 0,
          edits: 0,
          last_progress_ms: 1,
        }),
    ],
    [
      'an active attempt in another phase than the task',
      (v) => void (at(v, 2, 'attempt').phase = 'claimed'),
    ],
    [
      'an in-flight task without an active attempt',
      (v) => {
        at(v, 2, 'attempt').active = false;
        at(v, 2, 'attempt').phase = 'completed';
      },
    ],
    [
      'an active attempt in a finished phase',
      (v) => {
        v.items[1].status = 'running';
        v.items[1].receipt_status = null;
        at(v, 1, 'attempt').active = true;
      },
    ],
    [
      'a receipt that contradicts the task status',
      (v) => void (v.items[1].receipt_status = 'failed'),
    ],
    [
      'a receipt on an active attempt',
      (v) => void (v.items[2].receipt_status = 'completed'),
    ],
    ['a task that is its own parent', (v) => void (v.items[2].parent = id(2))],
    [
      'duplicate owned paths',
      (v) => void (v.items[2].paths = [`${studiolo}/web`, `${studiolo}/web`]),
    ],
  ])('rejects %s', (_, mutate) => rejects(mutate));

  it('accepts an idle queued item with a parent and no attempt', () => {
    const value = variant((v) => {
      v.items[0] = { ...draft(), status: 'queued', parent: id(1) };
    });
    expect(parseBoardOverview(value).items[0]).toMatchObject({
      status: 'queued',
      parent: id(1),
      attempt: null,
    });
  });

  it.each<[string, (value: Sample) => void]>([
    ['a duplicate item', (v) => void v.items.push(running())],
    ['items out of (project, id) order', (v) => void v.items.reverse()],
    [
      'an event for a task missing from items',
      (v) => void v.events.push(event(id(7), '2026-09-22T10:00:00Z')),
    ],
    [
      'an event whose project differs from its item',
      (v) => void (v.events[0].project = other),
    ],
    ['a finished item without window events', (v) => void v.events.shift()],
  ])('rejects %s', (_, mutate) => rejects(mutate));

  it('orders projects by code point like the server does', () => {
    const fullwidth = '项目/！';
    const emoji = '项目/\u{1f600}';
    const pair = (first: Json, second: Json) =>
      variant((v) => {
        v.items = [first, second];
        v.events = [];
      });
    expect(
      parseBoardOverview(pair(draft(1, fullwidth), draft(2, emoji))).items.map(
        (entry) => entry.project,
      ),
    ).toEqual([fullwidth, emoji]);
    expect(() =>
      parseBoardOverview(pair(draft(2, emoji), draft(1, fullwidth))),
    ).toThrow(BOARD_OVERVIEW_INVALID);
  });

  it('reports truncation flags and relaxes only the checks they explain', () => {
    const cappedItems = variant((v) => {
      v.items_truncated = true;
      v.events.push(event(id(7), '2026-09-22T10:00:00Z'));
    });
    const decodedItems = parseBoardOverview(cappedItems);
    expect(decodedItems.itemsTruncated).toBe(true);
    expect(decodedItems.events.at(-1)?.taskId).toBe(id(7));

    const cappedEvents = variant((v) => {
      v.events_truncated = true;
      v.events.shift();
    });
    const decodedEvents = parseBoardOverview(cappedEvents);
    expect(decodedEvents.eventsTruncated).toBe(true);
    expect(decodedEvents.items[1].status).toBe('succeeded');

    // 截断标志只解释缺失，不放宽别的检查。
    expect(() =>
      parseBoardOverview(
        variant((v) => {
          v.items_truncated = true;
          v.events[0].project = other;
        }),
      ),
    ).toThrow(BOARD_OVERVIEW_INVALID);
  });

  it('enforces the 500 item and 1000 event caps', () => {
    const drafts = (count: number) =>
      Array.from({ length: count }, (_, index) => draft(index + 1));
    const manyItems = (count: number) =>
      variant((v) => {
        v.items = drafts(count);
        v.items_truncated = true;
        v.events = [];
      });
    expect(parseBoardOverview(manyItems(500)).items).toHaveLength(500);
    expect(() => parseBoardOverview(manyItems(501))).toThrow(
      BOARD_OVERVIEW_INVALID,
    );
    const manyEvents = (count: number) =>
      variant((v) => {
        v.events = Array.from({ length: count }, (_, index) =>
          event(
            id(2),
            new Date(Date.UTC(2026, 8, 22, 0, 0, index)).toISOString(),
          ),
        );
        v.items = [running()];
        v.events_truncated = true;
      });
    expect(parseBoardOverview(manyEvents(1000)).events).toHaveLength(1000);
    expect(() => parseBoardOverview(manyEvents(1001))).toThrow(
      BOARD_OVERVIEW_INVALID,
    );
  });
});

describe('fetchBoardOverview', () => {
  let uninstall: (() => void) | null = null;
  afterEach(() => {
    uninstall?.();
    uninstall = null;
  });

  it('reads through the session gate with the requested window', async () => {
    const seen: string[] = [];
    uninstall = installSessionReader(async (path) => {
      seen.push(path);
      return sample();
    });
    const overview = await fetchBoardOverview(24, new AbortController().signal);
    expect(seen).toEqual(['/api/v1/board/overview?window_hours=24']);
    expect(overview.items.map((entry) => entry.id)).toEqual([
      id(3),
      id(1),
      id(2),
    ]);
  });

  it('rejects a response whose window differs from the request', async () => {
    uninstall = installSessionReader(async () => sample());
    await expect(
      fetchBoardOverview(1, new AbortController().signal),
    ).rejects.toThrow(BOARD_OVERVIEW_INVALID);
  });

  it('refuses windows outside 1–168 whole hours before any request', async () => {
    const seen: string[] = [];
    uninstall = installSessionReader(async (path) => {
      seen.push(path);
      return sample();
    });
    for (const hours of [0, 169, 1.5, Number.NaN, -1])
      await expect(
        fetchBoardOverview(hours, new AbortController().signal),
      ).rejects.toBeInstanceOf(RangeError);
    expect(seen).toEqual([]);
  });

  it('propagates server refusals and names only 404 as disabled', async () => {
    uninstall = installSessionReader(async () => {
      throw new ApiError('请求失败（404）', 404);
    });
    const failure = await fetchBoardOverview(
      24,
      new AbortController().signal,
    ).catch((error: unknown) => error);
    expect(boardOverviewDisabled(failure)).toBe(true);
    for (const status of [401, 429, 503])
      expect(boardOverviewDisabled(new ApiError('失败', status))).toBe(false);
    expect(boardOverviewDisabled(new Error('黑板不可用'))).toBe(false);
  });
});
