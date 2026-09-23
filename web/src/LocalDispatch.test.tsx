import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { LocalDispatch } from './LocalDispatch';

const project = '项目/Studiolo';
const client = {
  id: 'codex-local',
  provider: 'codex',
  mode: 'existing_session',
  project,
  cwd: '/tmp/studiolo-project',
  ready: true,
  reason: '',
};
const delivery = {
  id: 'delivery-1',
  client_id: client.id,
  task_id: 'TASK-1',
  state: 'sent',
  detail: '',
  created_ms: 1000,
  updated_ms: 1001,
};
function response(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
function reads(input: RequestInfo | URL) {
  return String(input).includes('/v1/clients')
    ? response({ clients: [client] })
    : response({ deliveries: [] });
}
async function mount(selectedProject = project) {
  let view!: ReturnType<typeof render>;
  await act(async () => {
    view = render(<LocalDispatch project={selectedProject} />);
  });
  fireEvent.click(screen.getByText('本机 Codex / Claude Code'));
  return view;
}
async function connect() {
  fireEvent.change(screen.getByLabelText('配对码'), {
    target: { value: 'local-pairing-code' },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '连接本机' }));
  });
}
function fillTask() {
  fireEvent.change(screen.getByLabelText('标题'), {
    target: { value: '核对文档' },
  });
  fireEvent.change(screen.getByLabelText('目标'), {
    target: { value: '读取并总结当前设计' },
  });
  fireEvent.change(screen.getByLabelText('验收条件（每行一条）'), {
    target: { value: ' 列出结论\n\n标明依据 ' },
  });
}
function postCalls(fetch: ReturnType<typeof vi.fn>) {
  return fetch.mock.calls.filter(
    (call) => (call[1] as RequestInit | undefined)?.method === 'POST',
  );
}
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it('starts collapsed and connects only to loopback without storing credentials', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL) => reads(input));
  const storage = vi.spyOn(Storage.prototype, 'setItem');
  vi.stubGlobal('fetch', fetch);
  const view = render(<LocalDispatch project={project} />);
  expect(view.container.querySelector('details')).not.toHaveAttribute('open');
  expect(fetch).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText('本机 Codex / Claude Code'));
  expect(screen.getByLabelText('配对码')).toHaveAttribute('type', 'password');
  await connect();
  expect(screen.getByRole('button', { name: '断开本机' })).toBeInTheDocument();
  expect(storage).not.toHaveBeenCalled();
  expect(fetch).toHaveBeenCalledTimes(2);
  for (const [url, init] of fetch.mock.calls as unknown as [
    string,
    RequestInit,
  ][]) {
    expect(url).toMatch(/^http:\/\/127\.0\.0\.1:4318\/v1\//);
    expect(url).not.toContain('local-pairing-code');
    expect(init).toMatchObject({
      credentials: 'omit',
      redirect: 'error',
      cache: 'no-store',
      headers: {
        Authorization: 'Bearer local-pairing-code',
        'Content-Type': 'application/json',
      },
    });
    expect(init.signal).toBeInstanceOf(AbortSignal);
  }
});

it.each([
  'http://localhost:4318',
  'https://127.0.0.1:4318',
  'http://127.0.0.1:4318@evil.test',
  'http://127.0.0.1:4318/?token=a',
  'http://127.0.0.1:65536',
  'http://127.0.0.2:4318',
])(
  'rejects an address outside the explicit loopback contract: %s',
  async (address) => {
    const fetch = vi.fn();
    vi.stubGlobal('fetch', fetch);
    await mount();
    fireEvent.change(screen.getByLabelText('本机地址'), {
      target: { value: address },
    });
    await connect();
    expect(screen.getByRole('alert')).toHaveTextContent(
      'http://127.0.0.1:端口',
    );
    expect(fetch).not.toHaveBeenCalled();
  },
);

it('filters other projects and prevents dispatch to an unavailable client with a reason', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) =>
      String(input).includes('/v1/clients')
        ? response({
            clients: [
              client,
              { ...client, id: 'foreign', project: '项目/Other' },
              {
                ...client,
                id: 'claude-local',
                provider: 'claude',
                ready: false,
                reason: '请在 Claude Code 打开本机通道',
              },
            ],
          })
        : response({ deliveries: [] }),
    ),
  );
  await mount();
  await connect();
  expect(
    screen.queryByRole('option', { name: /foreign/ }),
  ).not.toBeInTheDocument();
  fireEvent.change(screen.getByLabelText('执行客户端'), {
    target: { value: 'claude-local' },
  });
  expect(screen.getByRole('status')).toHaveTextContent(
    '请在 Claude Code 打开本机通道',
  );
  expect(screen.getByRole('button', { name: '创建并通知本机' })).toBeDisabled();
});

it('creates a read-only task with explicit criteria and distinguishes notification from execution', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) =>
    init?.method === 'POST'
      ? response({ task_id: delivery.task_id, delivery })
      : reads(input),
  );
  vi.stubGlobal('fetch', fetch);
  await mount();
  await connect();
  fillTask();
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '创建并通知本机' }));
  });
  const calls = postCalls(fetch);
  expect(calls).toHaveLength(1);
  expect(calls[0][0]).toBe('http://127.0.0.1:4318/v1/tasks');
  expect(JSON.parse(calls[0][1].body as string)).toEqual({
    client_id: client.id,
    request_id: expect.any(String),
    title: '核对文档',
    goal: '读取并总结当前设计',
    acceptance_criteria: ['列出结论', '标明依据'],
  });
  expect(screen.getByText('已通知（尚未证明执行）')).toBeInTheDocument();
  expect(screen.getByLabelText('标题')).toHaveValue('');
});

it('notifies an existing task without creating a new task definition', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) =>
    init?.method === 'POST' ? response(delivery) : reads(input),
  );
  vi.stubGlobal('fetch', fetch);
  await mount();
  await connect();
  fireEvent.change(screen.getByLabelText('派发内容'), {
    target: { value: 'existing' },
  });
  fireEvent.change(screen.getByLabelText('已有工作项 ID'), {
    target: { value: '  TASK-1  ' },
  });
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '通知本机' }));
  });
  const [url, init] = postCalls(fetch)[0];
  expect(url).toBe('http://127.0.0.1:4318/v1/dispatch');
  expect(JSON.parse(init.body as string)).toEqual({
    client_id: client.id,
    task_id: 'TASK-1',
    request_id: expect.any(String),
  });
});

it('blocks concurrent clicks and retries an uncertain response with the same request id', async () => {
  const first = deferred<Response>();
  let posts = 0;
  const fetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    if (init?.method !== 'POST') return Promise.resolve(reads(input));
    posts += 1;
    return posts === 1
      ? first.promise
      : Promise.resolve(response({ task_id: delivery.task_id, delivery }));
  });
  vi.stubGlobal('fetch', fetch);
  await mount();
  await connect();
  fillTask();
  const button = screen.getByRole('button', { name: '创建并通知本机' });
  act(() => {
    fireEvent.click(button);
    fireEvent.click(button);
  });
  expect(postCalls(fetch)).toHaveLength(1);
  await act(async () => {
    first.resolve(response({ error: 'private backend details' }, 503));
  });
  expect(screen.getByLabelText('标题')).toHaveValue('核对文档');
  expect(screen.getByRole('alert')).not.toHaveTextContent(
    'private backend details',
  );
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '创建并通知本机' }));
  });
  const calls = postCalls(fetch);
  expect(calls).toHaveLength(2);
  expect(JSON.parse(calls[0][1].body as string).request_id).toBe(
    JSON.parse(calls[1][1].body as string).request_id,
  );
});

it('polls every five seconds and disables dispatch if a client is revoked', async () => {
  let ready = true;
  const fetch = vi.fn(async (input: RequestInfo | URL) =>
    String(input).includes('/v1/clients')
      ? response({
          clients: [
            { ...client, ready, reason: ready ? '' : '客户端登记已撤销' },
          ],
        })
      : response({
          deliveries: [
            { ...delivery, state: 'completed', task_status: 'succeeded' },
          ],
        }),
  );
  vi.stubGlobal('fetch', fetch);
  await mount();
  await connect();
  expect(screen.getByText('已核对黑板终态')).toBeInTheDocument();
  expect(screen.getByText('黑板状态：succeeded')).toBeInTheDocument();
  ready = false;
  await act(async () => {
    await vi.advanceTimersByTimeAsync(5000);
  });
  expect(fetch).toHaveBeenCalledTimes(4);
  expect(screen.getByRole('button', { name: '创建并通知本机' })).toBeDisabled();
  expect(screen.getByRole('status')).toHaveTextContent('客户端登记已撤销');
});

it('aborts pending requests on project changes and ignores late responses', async () => {
  const late = deferred<Response>();
  const signals: AbortSignal[] = [];
  const fetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    signals.push(init!.signal as AbortSignal);
    return String(input).includes('/v1/clients')
      ? late.promise
      : Promise.resolve(response({ deliveries: [] }));
  });
  vi.stubGlobal('fetch', fetch);
  const view = await mount();
  await connect();
  view.rerender(<LocalDispatch project="项目/Other" />);
  expect(signals[0].aborted).toBe(true);
  await act(async () => {
    late.resolve(response({ clients: [client] }));
  });
  fireEvent.click(screen.getByText('本机 Codex / Claude Code'));
  expect(screen.getByLabelText('配对码')).toHaveValue('');
  expect(
    screen.queryByRole('button', { name: '断开本机' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '连接本机' })).toBeInTheDocument();
});

it('cancels connection and cannot be reconnected by the old response', async () => {
  const late = deferred<Response>();
  vi.stubGlobal(
    'fetch',
    vi.fn((input: RequestInfo | URL) =>
      String(input).includes('/v1/clients')
        ? late.promise
        : Promise.resolve(response({ deliveries: [] })),
    ),
  );
  await mount();
  await connect();
  fireEvent.click(screen.getByRole('button', { name: '取消连接' }));
  await act(async () => {
    late.resolve(response({ clients: [client] }));
  });
  expect(screen.getByRole('button', { name: '连接本机' })).toBeInTheDocument();
  expect(screen.getByLabelText('配对码')).toHaveValue('');
});

it('disconnects during dispatch and discards the late mutation result and polling', async () => {
  const late = deferred<Response>();
  let mutationSignal: AbortSignal | undefined;
  const fetch = vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
    if (init?.method === 'POST') {
      mutationSignal = init.signal as AbortSignal;
      return late.promise;
    }
    return Promise.resolve(reads(input));
  });
  vi.stubGlobal('fetch', fetch);
  await mount();
  await connect();
  fillTask();
  fireEvent.click(screen.getByRole('button', { name: '创建并通知本机' }));
  fireEvent.click(screen.getByRole('button', { name: '断开本机' }));
  expect(mutationSignal?.aborted).toBe(true);
  await act(async () => {
    late.resolve(response({ task_id: delivery.task_id, delivery }));
  });
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(fetch).toHaveBeenCalledTimes(3);
  expect(screen.queryByText(/已登记通知/)).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '连接本机' })).toBeInTheDocument();
});

it('times out a stalled request and exposes only a safe error', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(
      (_input: RequestInfo | URL, init?: RequestInit) =>
        new Promise<Response>((_resolve, reject) => {
          init?.signal?.addEventListener(
            'abort',
            () => reject(new Error('secret connection details')),
            { once: true },
          );
        }),
    ),
  );
  await mount();
  await connect();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(10_000);
  });
  expect(screen.getByRole('alert')).toHaveTextContent('无法确认本机连接结果');
  expect(screen.getByRole('alert')).not.toHaveTextContent(
    'secret connection details',
  );
  expect(screen.getByRole('button', { name: '连接本机' })).toBeEnabled();
});

it('lists notification records with a machine-readable time first and — for a time it cannot show', async () => {
  const updated = Date.UTC(2026, 8, 23, 1, 2, 3);
  vi.stubGlobal(
    'fetch',
    vi.fn(async (input: RequestInfo | URL) =>
      String(input).includes('/v1/clients')
        ? response({ clients: [client] })
        : response({
            deliveries: [
              { ...delivery, updated_ms: updated },
              {
                ...delivery,
                id: 'delivery-2',
                state: 'uncertain',
                updated_ms: 8.64e15 + 1,
              },
            ],
          }),
    ),
  );
  await mount();
  await connect();
  const records = screen.getByRole('region', { name: '本机通知记录' });
  const [first, second] = within(records).getAllByRole('listitem');
  const time = first.querySelector('time');
  expect(first.firstElementChild).toBe(time);
  expect(time).toHaveAttribute('datetime', new Date(updated).toISOString());
  expect(within(first).getByText('已通知（尚未证明执行）')).toBeInTheDocument();
  expect(second.querySelector('time')).toBeNull();
  expect(second.firstElementChild).toHaveTextContent('—');
  expect(
    within(second).getByText('投递结果不明，请核查本机任务'),
  ).toBeInTheDocument();
});
