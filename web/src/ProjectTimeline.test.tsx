import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ProjectTimeline } from './ProjectTimeline';
import { CollabView } from './views/CollabView';

const project = '项目/Studiolo';
const other = '项目/另一个';
const now = Date.UTC(2026, 8, 20, 10);
const projects = [
  { root: project, name: 'Studiolo' },
  { root: other, name: '另一个' },
];
const task = '00000000000000000000000001';
function response(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
function query(input: RequestInfo | URL) {
  return new URL(String(input), 'http://localhost').searchParams;
}
function event(offset: string, at: string, action = '任务事件') {
  return {
    offset,
    at,
    task_id: task,
    action,
    phase: 'outcome',
    attempt_id: `A-${task}`,
  };
}
function page(input: RequestInfo | URL) {
  const params = query(input);
  return {
    project: params.get('project'),
    from: params.get('from'),
    to: params.get('to'),
    events: [] as ReturnType<typeof event>[],
    logs: [] as { path: string; title: string; date: string }[],
    undated_logs: 0,
    next: null as string | null,
  };
}
function mockReads() {
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
    void init;
    return response(
      String(input).startsWith('/api/v1/board/timeline?') ? page(input) : [],
    );
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}
async function mount() {
  await act(async () => {
    render(<ProjectTimeline project={project} />);
  });
}
async function click(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name }));
  });
}
/** 回放是协作页的「时间线」子标签，实时黑板在「任务与回执」子标签。 */
async function clickTab(name: string) {
  await act(async () => {
    fireEvent.click(screen.getByRole('tab', { name }));
  });
}
/**
 * 直接落在「任务与回执」且项目已选：这三个标签要先选具体项目（设计规范第 7 节），
 * 缺省的拓扑标签会另读跨项目汇总，不在这里的请求计数之内。
 */
async function mountBoard() {
  window.location.hash = `#view=collab&panel=board&project=${encodeURIComponent(project)}`;
  await act(async () => {
    render(<CollabView projects={projects} />);
  });
  await clickTab('任务与回执');
}
async function change(element: HTMLElement, value: string) {
  await act(async () => {
    fireEvent.change(element, { target: { value } });
  });
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(now);
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('switches between the live board and replay without making execution requests or polling inactive modes', async () => {
  const fetch = mockReads();
  await mountBoard();
  expect(screen.getByText('无活动')).toBeInTheDocument();
  expect(fetch.mock.calls).toHaveLength(1);
  await click('项目时间回放');
  expect(
    screen.getByRole('region', { name: '项目时间回放' }),
  ).toBeInTheDocument();
  expect(screen.queryByText('无活动')).not.toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(45_000);
  });
  expect(fetch.mock.calls).toHaveLength(2);
  await clickTab('任务与回执');
  expect(screen.getByText('无活动')).toBeInTheDocument();
  expect(
    screen.queryByRole('region', { name: '项目时间回放' }),
  ).not.toBeInTheDocument();
  expect(fetch.mock.calls).toHaveLength(3);
  expect(
    fetch.mock.calls.every(([url]) => String(url).startsWith('/api/v1/board')),
  ).toBe(true);
});
it('requests an explicit UTC window, sends the exact cursor and resets pagination when changing the window or refreshing', async () => {
  const fetch = mockReads();
  fetch.mockImplementation(async (input) =>
    response({
      ...page(input),
      next: query(input).has('after') ? null : '9007199254740993',
    }),
  );
  await mount();
  expect(query(fetch.mock.calls[0][0]).get('from')).toBe(
    '2026-09-19T10:00:00.000Z',
  );
  expect(query(fetch.mock.calls[0][0]).get('to')).toBe(
    '2026-09-20T10:00:00.000Z',
  );
  expect(
    screen.getByText('本页尚无匹配活动，请继续下一页。'),
  ).toBeInTheDocument();
  await click('下一页事件');
  expect(query(fetch.mock.calls.at(-1)![0]).get('after')).toBe(
    '9007199254740993',
  );
  await change(screen.getByRole('combobox', { name: '时间范围' }), '7');
  expect(query(fetch.mock.calls.at(-1)![0]).get('from')).toBe(
    '2026-09-13T10:00:00.000Z',
  );
  expect(query(fetch.mock.calls.at(-1)![0]).has('after')).toBe(false);
  await click('下一页事件');
  await click('重新核对回放');
  expect(query(fetch.mock.calls.at(-1)![0]).has('after')).toBe(false);
  await change(screen.getByRole('combobox', { name: '时间范围' }), '30');
  expect(query(fetch.mock.calls.at(-1)![0]).get('from')).toBe(
    '2026-08-21T10:00:00.000Z',
  );
});
it('scrubs events and UTC-dated log pointers locally without fetching or inventing a historical board snapshot', async () => {
  const fetch = mockReads();
  fetch.mockImplementation(async (input) =>
    response({
      ...page(input),
      events: [
        event('1', '2026-09-19T16:00:00Z', '较早事件'),
        event('2', '2026-09-20T09:00:00Z', '较晚事件'),
      ],
      logs: [
        {
          path: `${project}/02-日志/2026-09-20.md`,
          title: '日志指针',
          date: '2026-09-20',
        },
      ],
      undated_logs: 3,
    }),
  );
  await mount();
  expect(screen.getByText(/较晚事件/)).toBeInTheDocument();
  expect(screen.getByText(/日志指针/)).toBeInTheDocument();
  expect(screen.getByText(/另有 3 篇日志/)).toBeInTheDocument();
  expect(
    screen.getByText(/不将当前工作项状态当作历史快照/),
  ).toBeInTheDocument();
  await change(screen.getByRole('slider', { name: '回放时间轴' }), '250');
  expect(screen.getByText(/较早事件/)).toBeInTheDocument();
  expect(screen.queryByText(/较晚事件/)).not.toBeInTheDocument();
  expect(screen.queryByText(/日志指针/)).not.toBeInTheDocument();
  await change(screen.getByRole('slider', { name: '回放时间轴' }), '0');
  expect(screen.getByText('所选范围无活动。')).toBeInTheDocument();
  expect(fetch).toHaveBeenCalledTimes(1);
});
it('rejects a non-advancing next cursor and does not offer another page', async () => {
  const fetch = mockReads();
  fetch.mockImplementation(async (input) =>
    response({ ...page(input), next: '1000' }),
  );
  await mount();
  await click('下一页事件');
  expect(screen.getByRole('alert')).toHaveTextContent('游标未前进');
  expect(
    screen.queryByRole('button', { name: '下一页事件' }),
  ).not.toBeInTheDocument();
});
it('rejects replayed events before the requested cursor even on a final page', async () => {
  const fetch = mockReads();
  fetch.mockImplementation(async (input) =>
    response(
      query(input).has('after')
        ? {
            ...page(input),
            events: [event('1000', '2026-09-20T09:00:00Z', '重复旧事件')],
          }
        : { ...page(input), next: '1000' },
    ),
  );
  await mount();
  await click('下一页事件');
  expect(screen.getByRole('alert')).toBeInTheDocument();
  expect(screen.queryByText(/重复旧事件/)).not.toBeInTheDocument();
});
it('clears prior events when refresh fails instead of presenting an empty timeline', async () => {
  const fetch = mockReads();
  fetch
    .mockImplementationOnce(async (input) =>
      response({
        ...page(input),
        events: [event('1', '2026-09-20T09:00:00Z', '旧事件')],
      }),
    )
    .mockResolvedValueOnce(response({ error: '回放不可用' }, 503));
  await mount();
  expect(screen.getByText(/旧事件/)).toBeInTheDocument();
  await click('重新核对回放');
  expect(screen.getByRole('alert')).toBeInTheDocument();
  expect(screen.queryByText(/旧事件/)).not.toBeInTheDocument();
  expect(screen.queryByText('所选范围无活动。')).not.toBeInTheDocument();
});
it('project changes abort and discard a late response from the old project', async () => {
  let finish!: (value: Response) => void;
  let oldRequest: RequestInfo | URL = '';
  const fetch = mockReads();
  fetch.mockImplementation(async (input) => {
    if (!String(input).startsWith('/api/v1/board/timeline?'))
      return response([]);
    if (query(input).get('project') === project) {
      oldRequest = input;
      return new Promise<Response>((resolve) => {
        finish = resolve;
      });
    }
    return response(page(input));
  });
  await mountBoard();
  await click('项目时间回放');
  const first = fetch.mock.calls.find(([input]) =>
    String(input).startsWith('/api/v1/board/timeline?'),
  )!;
  await change(screen.getByRole('combobox', { name: '项目' }), other);
  expect(first[1]?.signal?.aborted).toBe(true);
  expect(query(fetch.mock.calls.at(-1)![0]).get('project')).toBe(other);
  await act(async () => {
    finish(
      response({
        ...page(oldRequest),
        events: [event('1', '2026-09-20T09:00:00Z', '旧项目事件')],
      }),
    );
  });
  expect(screen.queryByText(/旧项目事件/)).not.toBeInTheDocument();
  expect(screen.getByText('所选范围无活动。')).toBeInTheDocument();
});
it('rejects a fresh but cross-project response rather than showing its event metadata', async () => {
  const fetch = mockReads();
  fetch.mockImplementation(async (input) =>
    response({
      ...page(input),
      project: other,
      events: [event('1', '2026-09-20T09:00:00Z', '其他项目事件')],
    }),
  );
  await mount();
  expect(screen.getByRole('alert')).toHaveTextContent('归属无效');
  expect(screen.queryByText(/其他项目事件/)).not.toBeInTheDocument();
});
