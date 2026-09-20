import { expect, it } from 'vitest';
import { timelinePage } from './projectTimelineModel';

const project = '项目/Studiolo';
const from = '2026-09-19T10:00:00.000Z';
const to = '2026-09-20T10:00:00.000Z';
const task = '00000000000000000000000001';
function event(offset = '1', at = '2026-09-20T09:00:00Z') {
  return {
    offset,
    at,
    task_id: task,
    action: 'runner.call',
    phase: 'outcome',
    attempt_id: `A-${task}`,
  };
}
function page() {
  return {
    project,
    from,
    to,
    events: [event()],
    logs: [
      {
        path: `${project}/02-日志/2026-09-20.md`,
        title: '当日记录',
        date: '2026-09-20',
      },
    ],
    undated_logs: 2,
    next: null,
  };
}

it('preserves exact project, time bounds, dated pointers and u64 offsets without number conversion', () => {
  const value = {
    ...page(),
    events: [event('9007199254740993')],
    next: '18446744073709551615',
  };
  expect(timelinePage(value, project, from, to)).toEqual(value);
  expect(
    timelinePage(
      { ...page(), events: [event('1', from), event('2', to)] },
      project,
      from,
      to,
    ).events,
  ).toHaveLength(2);
});
it('permits empty matching event pages with a later global scan cursor', () => {
  expect(
    timelinePage(
      { ...page(), events: [], logs: [], next: '1000' },
      project,
      from,
      to,
    ),
  ).toMatchObject({ events: [], next: '1000' });
  expect(
    timelinePage(
      { ...page(), events: [event('5')], next: '1000' },
      project,
      from,
      to,
    ).next,
  ).toBe('1000');
});
it('rejects cross-project pages, logs outside the project and mismatched requested windows', () => {
  for (const change of [
    { project: '项目/别处' },
    { from: '2026-09-18T10:00:00.000Z' },
    { to: '2026-09-21T10:00:00.000Z' },
    { logs: [{ ...page().logs[0], path: '项目/别处/02-日志/2026-09-20.md' }] },
    {
      logs: [
        { ...page().logs[0], path: `${project}/02-日志/../03-记忆/secret.md` },
      ],
    },
  ])
    expect(() =>
      timelinePage({ ...page(), ...change }, project, from, to),
    ).toThrow();
});
it('rejects non-RFC3339 or out-of-window event timestamps', () => {
  for (const at of [
    'yesterday',
    '2026-09-20',
    '2026-09-20T09:00:00',
    '2026-09-19T09:59:59Z',
    '2026-09-20T10:00:01Z',
    '2026-09-20T25:00:00Z',
  ])
    expect(() =>
      timelinePage({ ...page(), events: [event('1', at)] }, project, from, to),
    ).toThrow();
});
it('rejects invalid and reversed windows even when their arrays are empty', () => {
  for (const [start, end] of [
    ['bad', to],
    [from, 'bad'],
    [to, from],
    ['2026-08-01T00:00:00Z', to],
  ])
    expect(() =>
      timelinePage(
        { ...page(), from: start, to: end, events: [], logs: [] },
        project,
        start,
        end,
      ),
    ).toThrow();
});
it('rejects normalized impossible dates rather than assigning them a different day', () => {
  const start = '2026-02-01T00:00:00Z';
  const end = '2026-03-03T00:00:00Z';
  expect(() =>
    timelinePage(
      {
        ...page(),
        from: start,
        to: end,
        events: [],
        logs: [
          {
            path: `${project}/02-日志/2026-02-30.md`,
            title: '非法日期',
            date: '2026-02-30',
          },
        ],
      },
      project,
      start,
      end,
    ),
  ).toThrow();
  expect(() =>
    timelinePage(
      {
        ...page(),
        from: start,
        to: end,
        events: [event('1', '2026-02-30T12:00:00Z')],
        logs: [],
      },
      project,
      start,
      end,
    ),
  ).toThrow();
});
it('requires strictly increasing canonical u64 offsets and a cursor not behind the returned events', () => {
  for (const events of [
    [event('2'), event('1')],
    [event('1'), event('1')],
    [event('00')],
    [event('-1')],
    [event('18446744073709551616')],
  ])
    expect(() =>
      timelinePage({ ...page(), events }, project, from, to),
    ).toThrow();
  for (const next of ['00', '-1', '18446744073709551616', 'not-a-cursor', '1'])
    expect(() =>
      timelinePage(
        { ...page(), events: [event('2')], next },
        project,
        from,
        to,
      ),
    ).toThrow();
});
it('rejects unknown fields, phases, malformed counts and oversized pages', () => {
  for (const change of [
    { raw: 'body' },
    { events: [{ ...event(), raw: 'body' }] },
    { events: [{ ...event(), phase: 'completed' }] },
    { logs: [{ ...page().logs[0], content: 'body' }] },
    { undated_logs: -1 },
    { undated_logs: 0.5 },
    { undated_logs: '2' },
    { events: Array.from({ length: 1001 }, (_, i) => event(String(i + 1))) },
    { logs: Array.from({ length: 367 }, () => page().logs[0]) },
  ])
    expect(() =>
      timelinePage({ ...page(), ...change }, project, from, to),
    ).toThrow();
});
