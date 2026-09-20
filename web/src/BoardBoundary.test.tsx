import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { BoardBoundary } from './BoardBoundary';

const project = '项目/Studiolo';
const projects = [{ root: project, name: 'Studiolo' }];
const hash = 'a'.repeat(64);
const now = Date.UTC(2026, 8, 20, 10);
const id = (n: number) => String(n).padStart(26, '0');
function row(
  n = 1,
  status = 'running',
  parent: string | null = null,
  depth = 0,
) {
  const active = [
    'claimed',
    'running',
    'cancel_requested',
    'committing',
  ].includes(status);
  const attempted =
    active || !['draft', 'queued', 'needs_reconfirm'].includes(status);
  const attemptId = `A-${id(n + 100)}`;
  return {
    task: {
      task_id: id(n),
      spec: {
        role: 'builder',
        adapter: 'consumer',
        input_paths: [],
        allow_web: [],
        allow_db: [],
        retry_policy: { max_attempts: 1, backoff_seconds: 0, retry_on: [] },
        timeout: 600,
        project,
        parent,
        title: `工作项 ${n}`,
        instruction: null,
      },
      status,
      active_attempt: active ? attemptId : null,
      last_attempt: attempted && !active ? attemptId : null,
      input_snapshot: attempted ? hash : null,
    },
    board: {
      task_id: id(n),
      definition: {
        acceptance_criteria: ['契约通过'],
        files_owned: ['项目/Studiolo/03-记忆'],
        max_children: 8,
        policy_hash: hash,
        parent,
        artifact_target: null,
      },
      depth,
      created_by_instance: 'creator',
      handoff: null,
      receipt: null as unknown,
    },
    candidate_stages: [],
    candidate_receipt: null,
    uses_old_policy: false,
    holder: attempted ? 'consumer-instance' : null,
    progress: attempted
      ? { tool_calls: 7, edits: 2, last_progress_ms: now - 500 }
      : null,
    attempt: attempted
      ? {
          attempt_id: attemptId,
          task_id: id(n),
          fence: 42,
          phase: active ? status : 'completed',
          worker_id: 'consumer-instance',
          snapshot_id: hash,
          lease_until_ms: now + 60_000,
          runner_ref_json: null,
          terminal: null,
        }
      : null,
  };
}
function response(value: unknown, status = 200) {
  return new Response(JSON.stringify(value), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}
async function mount(available = projects) {
  let rendered: ReturnType<typeof render>;
  await act(async () => {
    rendered = render(<BoardBoundary projects={available} />);
  });
  return rendered!;
}
async function advance(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
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

it('renders the actual tree, holder, fence, lease, paths and pinned old policy', async () => {
  const child = row(2, 'running', id(1), 1);
  child.uses_old_policy = true;
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response([child, row(1, 'queued')])),
  );
  await mount();
  const parent = screen.getByRole('article', { name: '工作项 1' });
  expect(parent).toHaveTextContent('等待子项收尾');
  const children = screen.getByRole('list', { name: '工作项 1的子工作项' });
  const item = within(children).getByRole('article', { name: '工作项 2' });
  expect(item).toHaveTextContent('当前持有实例：consumer-instance');
  expect(item).toHaveTextContent('认领凭据（fence）42');
  expect(item.querySelector('time')).toHaveAttribute(
    'dateTime',
    new Date(now + 60_000).toISOString(),
  );
  expect(item).toHaveTextContent('项目/Studiolo/03-记忆');
  expect(item).toHaveTextContent('按旧版规则执行');
  expect(screen.getByText(/1 项租约有效/)).toBeInTheDocument();
});

it('shows queued work and only the backend needs_review status as awaiting review', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response([row(1, 'queued'), row(2, 'needs_review')])),
  );
  await mount();
  expect(screen.getByRole('article', { name: '工作项 1' })).toHaveTextContent(
    '待认领',
  );
  const reviewed = screen.getByRole('article', { name: '工作项 2' });
  expect(reviewed).toHaveTextContent('待审阅');
  expect(reviewed).toHaveTextContent('最近持有实例：consumer-instance');
  expect(reviewed).not.toHaveTextContent('当前持有实例：consumer-instance');
  expect(
    screen.queryByRole('button', { name: /批准/ }),
  ).not.toBeInTheDocument();
});

it('expires a displayed lease without waiting for the next server poll', async () => {
  const value = row();
  value.attempt!.lease_until_ms = now + 1000;
  const fetch = vi.fn(async () => response([value]));
  vi.stubGlobal('fetch', fetch);
  await mount();
  expect(screen.getByText(/1 项租约有效/)).toBeInTheDocument();
  await advance(1000);
  const item = screen.getByRole('article', { name: '工作项 1' });
  expect(item).toHaveTextContent('租约已到期，状态待核对');
  expect(item).toHaveTextContent('最近持有实例：consumer-instance');
  expect(item).not.toHaveTextContent('当前持有实例：consumer-instance');
  expect(screen.getByText(/0 项租约有效/)).toBeInTheDocument();
  expect(fetch).toHaveBeenCalledTimes(1);
});

it('renders structured receipt assessments, unlanded patches and closeout', async () => {
  const value = row(1, 'failed');
  value.attempt!.phase = 'failed';
  value.board.receipt = {
    status: 'failed',
    artifacts: [],
    conflicts: [],
    unlanded: [{ path: 'sources/unlanded.patch', sha256: hash }],
    acceptance: [{ criterion: '契约通过', result: 'unverified', evidence: [] }],
    remaining: ['等待服务器复测'],
    elapsed_ms: 3500,
    tool_calls: 7,
    learned: [],
    closeout: {
      learned: ['保留未落地补丁'],
      adr_required: true,
      rule_change_required: false,
      destinations: ['项目/Studiolo/03-记忆'],
    },
  };
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response([value])),
  );
  await mount();
  const item = screen.getByRole('article', { name: '工作项 1' });
  expect(item).toHaveTextContent('收尾回执 · 失败');
  expect(item).toHaveTextContent('契约通过：未验证');
  expect(item).toHaveTextContent('sources/unlanded.patch');
  expect(item).toHaveTextContent('等待服务器复测');
  expect(item).toHaveTextContent('需要 ADR：是');
});

it.each([401, 404, 503])(
  'clears previously current claims when the actual poll returns %i',
  async (status) => {
    const fetch = vi
      .fn()
      .mockResolvedValueOnce(response([row()]))
      .mockResolvedValueOnce(response({ error: '读取失败' }, status));
    vi.stubGlobal('fetch', fetch);
    await mount();
    expect(
      screen.getByRole('article', { name: '工作项 1' }),
    ).toBeInTheDocument();
    await advance(15_000);
    expect(screen.getByRole('alert')).toHaveTextContent('实时状态不可确认');
    expect(screen.queryByRole('article')).not.toBeInTheDocument();
    expect(screen.queryByText(/项租约有效/)).not.toBeInTheDocument();
    expect(screen.getByRole('alert')).not.toHaveTextContent('无活动');
    expect(fetch).toHaveBeenCalledTimes(2);
  },
);

it('clears old state on network failure and retry publishes only the new response', async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(response([row()]))
    .mockRejectedValueOnce(new Error('网络中断'))
    .mockResolvedValueOnce(response([]));
  vi.stubGlobal('fetch', fetch);
  await mount();
  await advance(15_000);
  expect(screen.getByRole('alert')).toHaveTextContent('网络中断');
  expect(screen.queryByRole('article')).not.toBeInTheDocument();
  await act(async () => {
    fireEvent.click(screen.getByRole('button', { name: '重新核对' }));
  });
  expect(screen.getByText('无活动')).toBeInTheDocument();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});

it('rejects a malformed poll rather than keeping old state or showing an empty board', async () => {
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(response([row()]))
    .mockResolvedValueOnce(response({ items: [] }));
  vi.stubGlobal('fetch', fetch);
  await mount();
  await advance(15_000);
  expect(screen.getByRole('alert')).toHaveTextContent('实时状态不可确认');
  expect(screen.queryByRole('article')).not.toBeInTheDocument();
  expect(screen.queryByText('无活动')).not.toBeInTheDocument();
});

it('times out a stalled poll, aborts its request and ignores a late successful response', async () => {
  let finish: (value: Response) => void = () => {};
  const pending = new Promise<Response>((resolve) => {
    finish = resolve;
  });
  const fetch = vi
    .fn()
    .mockResolvedValueOnce(response([row()]))
    .mockReturnValueOnce(pending);
  vi.stubGlobal('fetch', fetch);
  await mount();
  await advance(25_000);
  expect(screen.getByRole('alert')).toHaveTextContent('黑板读取超时');
  expect(fetch.mock.calls[1][1].signal.aborted).toBe(true);
  expect(screen.queryByRole('article')).not.toBeInTheDocument();
  await act(async () => {
    finish(response([row(2)]));
  });
  expect(screen.getByRole('alert')).toHaveTextContent('实时状态不可确认');
  expect(screen.queryByRole('article')).not.toBeInTheDocument();
});

it('shows no activity only after receiving a valid empty board', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => response([])),
  );
  await mount();
  expect(screen.getByText('无活动')).toBeInTheDocument();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});

it('aborts and discards the old project response when the project changes', async () => {
  let finish: (value: Response) => void = () => {};
  const fetch = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<Response>((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValueOnce(response([]));
  vi.stubGlobal('fetch', fetch);
  await mount([...projects, { root: '项目/另一个', name: '另一个' }]);
  await act(async () => {
    fireEvent.change(screen.getByRole('combobox'), {
      target: { value: '项目/另一个' },
    });
  });
  expect(fetch.mock.calls[0][1]?.signal?.aborted).toBe(true);
  await act(async () => {
    finish(response([row()]));
  });
  expect(screen.getByText('无活动')).toBeInTheDocument();
  expect(screen.queryByRole('article')).not.toBeInTheDocument();
});

it('unmount cancels the pending request and prevents future polling', async () => {
  const fetch = vi.fn<
    (input: RequestInfo | URL, init?: RequestInit) => Promise<Response>
  >(() => new Promise<Response>(() => {}));
  vi.stubGlobal('fetch', fetch);
  const rendered = await mount();
  rendered.unmount();
  await advance(60_000);
  expect(fetch.mock.calls[0][1]?.signal?.aborted).toBe(true);
  expect(fetch).toHaveBeenCalledTimes(1);
});

it('shows staged and committed candidate evidence separately from acceptance without execution fetches', async () => {
  const candidate = row(1, 'needs_review');
  Object.assign(candidate, {
    candidate_stages: [
      {
        attempt_id: `A-${id(101)}`,
        fence: 42,
        pointer: { id: hash, sha256: hash },
        reason: 'retained',
      },
    ],
    candidate_receipt: {
      manifest_path: '项目/Studiolo/03-记忆/manifest.json',
      manifest_revision: hash,
      artifacts: [
        {
          path: '项目/Studiolo/03-记忆/result.md',
          revision: hash,
          sha256: hash,
        },
      ],
    },
  });
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    void input;
    return response([candidate]);
  });
  vi.stubGlobal('fetch', fetch);
  await mount();
  expect(screen.getByText(/候选证据 · 1 个暂存记录/)).toBeInTheDocument();
  expect(
    screen.getByText('项目/Studiolo/03-记忆/result.md'),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/是否通过以逐项验收和收尾回执为准/),
  ).toBeInTheDocument();
  expect(
    fetch.mock.calls.every(([url]) => String(url).startsWith('/api/v1/board?')),
  ).toBe(true);
});
