import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ExecutionBoundary } from './ExecutionBoundary';
import { installExecutionWriter } from './executionApi';
import { ApiError } from './sessionHttp';
import {
  executionBoard,
  executionEvent,
  executionId,
  executionProject,
  executionReceipt,
  executionResponse,
  executionRun,
} from './test/executionFixtures';

const projects = [{ root: executionProject, name: 'Studiolo' }];
let uninstall: (() => void) | undefined;
let currentBoard = executionBoard();
let status = {
  configured: true,
  frozen: false,
  runs: [] as ReturnType<typeof executionRun>[],
};
function mockReads() {
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === '/api/v1/execution') return executionResponse(status);
    if (url.startsWith('/api/v1/board?'))
      return executionResponse([currentBoard]);
    if (url.startsWith('/api/v1/board/replay?'))
      return executionResponse({
        task: currentBoard,
        events: [executionEvent()],
        next: null,
      });
    return executionResponse({}, 404);
  });
  vi.stubGlobal('fetch', fetch);
  return fetch;
}
async function mount() {
  await act(async () => {
    render(<ExecutionBoundary projects={projects} />);
  });
}
async function click(button: HTMLElement) {
  await act(async () => {
    fireEvent.click(button);
  });
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(Date.UTC(2026, 8, 20, 10));
  currentBoard = executionBoard();
  status = { configured: true, frozen: false, runs: [] };
});
afterEach(() => {
  cleanup();
  uninstall?.();
  uninstall = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('unconfigured execution remains frozen and cannot start', async () => {
  status = { configured: false, frozen: true, runs: [] };
  mockReads();
  await mount();
  expect(screen.getByText('执行服务未配置 · 默认冻结')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '启动登记执行体' })).toBeDisabled();
  expect(
    screen.queryByRole('button', { name: '二次认证后解冻执行' }),
  ).not.toBeInTheDocument();
});
it('start uses registered task identity and refreshes authority without a second POST', async () => {
  const fetch = mockReads();
  const writer = vi.fn(async (body: unknown, signal: AbortSignal) => {
    void body;
    void signal;
    currentBoard = executionBoard('running');
    status.runs = [executionRun()];
    return executionRun();
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  expect(
    screen.queryByRole('textbox', { name: /shell|命令/ }),
  ).not.toBeInTheDocument();
  await click(screen.getByRole('button', { name: '启动登记执行体' }));
  expect(writer).toHaveBeenCalledTimes(1);
  expect(writer.mock.calls[0]?.[0]).toEqual({
    operation: 'start',
    task_id: executionId(),
    agent_type: 'codex',
    instance_id: 'registered-codex-instance',
  });
  expect(
    screen.getByText(/节奏检查 · 工具调用 5 次 · 编辑 2 次/),
  ).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '启动登记执行体' })).toBeDisabled();
  expect(
    fetch.mock.calls.filter(([path]) => path === '/api/v1/execution'),
  ).toHaveLength(2);
  expect(
    fetch.mock.calls.filter(([path]) =>
      String(path).startsWith('/api/v1/board/replay?'),
    ).length,
  ).toBeGreaterThanOrEqual(2);
});
it('unfreeze L3 rejection stays frozen, explains reauthentication and never retries', async () => {
  status.frozen = true;
  mockReads();
  const writer = vi.fn(async (body: unknown, signal: AbortSignal) => {
    void body;
    void signal;
    throw new ApiError('请先完成二次认证（One-time Code）', 409);
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  await click(screen.getByRole('button', { name: '二次认证后解冻执行' }));
  expect(screen.getByRole('alert')).toHaveTextContent(
    '请先在「账户」完成二次认证',
  );
  expect(screen.getByText('执行服务已冻结')).toBeInTheDocument();
  expect(screen.getByRole('button', { name: '启动登记执行体' })).toBeDisabled();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(writer).toHaveBeenCalledTimes(1);
});
it('a failed status poll removes stale runs and disables commands', async () => {
  currentBoard = executionBoard('running');
  status.runs = [executionRun()];
  const fetch = mockReads();
  await mount();
  expect(screen.getByRole('button', { name: '取消执行' })).toBeEnabled();
  fetch.mockImplementation(async (input: RequestInfo | URL) =>
    String(input) === '/api/v1/execution'
      ? executionResponse({}, 404)
      : String(input).startsWith('/api/v1/board/replay?')
        ? executionResponse({ task: currentBoard, events: [], next: null })
        : executionResponse([currentBoard]),
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(screen.getByRole('alert')).toHaveTextContent('当前状态不可确认');
  expect(
    screen.queryByRole('region', { name: '执行体进度' }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '取消执行' })).toBeDisabled();
});
it('cancel is a single controlled command and reconciles stopped state', async () => {
  currentBoard = executionBoard('running');
  status.runs = [executionRun()];
  mockReads();
  const writer = vi.fn(async (body: unknown, signal: AbortSignal) => {
    void body;
    void signal;
    currentBoard = executionBoard('cancelled');
    if (currentBoard.attempt) currentBoard.attempt.phase = 'cancelled';
    status.runs = [executionRun('stopped')];
    return { ok: true };
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  await click(screen.getByRole('button', { name: '取消执行' }));
  expect(writer.mock.calls[0]?.[0]).toEqual({
    operation: 'cancel',
    task_id: executionId(),
  });
  expect(screen.getByRole('button', { name: '取消执行' })).toBeDisabled();
});
it('review requires explicit JSON decisions and acknowledgment before posting the exact receipt', async () => {
  currentBoard = executionBoard('needs_review');
  status.runs = [executionRun('candidate_review')];
  mockReads();
  const writer = vi.fn(async (body: unknown, signal: AbortSignal) => {
    void body;
    void signal;
    currentBoard = executionBoard('succeeded');
    currentBoard.board.receipt = executionReceipt();
    return currentBoard;
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  const button = screen.getByRole('button', { name: '提交审阅回执' });
  expect(button).toBeDisabled();
  const input = screen.getByRole('textbox', { name: '验收回执 JSON' });
  expect(
    JSON.parse((input as HTMLTextAreaElement).value).acceptance[0].result,
  ).toBe('unverified');
  await click(screen.getByRole('checkbox'));
  await click(button);
  expect(screen.getByRole('alert')).toHaveTextContent('回执字段不完整');
  expect(writer).not.toHaveBeenCalled();
  fireEvent.change(input, {
    target: { value: JSON.stringify(executionReceipt()) },
  });
  expect(button).toBeDisabled();
  await click(screen.getByRole('checkbox'));
  await click(button);
  expect(writer).toHaveBeenCalledTimes(1);
  expect(writer.mock.calls[0]?.[0]).toEqual({
    operation: 'review',
    task_id: executionId(),
    receipt: executionReceipt(),
  });
  expect(
    screen.queryByRole('textbox', { name: '验收回执 JSON' }),
  ).not.toBeInTheDocument();
});
it('retained but unsealed candidate is shown as unaccepted and cannot be reviewed', async () => {
  currentBoard = executionBoard('failed');
  status.runs = [executionRun('candidate_retained')];
  mockReads();
  await mount();
  expect(screen.getByText(/候选留存，未验收/)).toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: '提交审阅回执' }),
  ).not.toBeInTheDocument();
});
it('audit replay follows exact server pagination with no raw content', async () => {
  const fetch = mockReads();
  fetch.mockImplementation(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === '/api/v1/execution') return executionResponse(status);
    if (url.startsWith('/api/v1/board?'))
      return executionResponse([currentBoard]);
    if (url.includes('after=100'))
      return executionResponse({
        task: currentBoard,
        events: [executionEvent('101')],
        next: null,
      });
    return executionResponse({
      task: currentBoard,
      events: Array.from({ length: 100 }, (_, i) =>
        executionEvent(String(i + 1)),
      ),
      next: '100',
    });
  });
  await mount();
  await click(screen.getByRole('button', { name: '下一页' }));
  expect(
    fetch.mock.calls.some(([url]) => String(url).endsWith('&after=100')),
  ).toBe(true);
  expect(screen.getByText(/当前页在 offset 100 之后/)).toBeInTheDocument();
  expect(
    screen.getByText('offset 101 · A-00000000000000000000000101'),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole('button', { name: '下一页' }),
  ).not.toBeInTheDocument();
});

it('a timed-out poll clears authority and cannot be revived by a late successful response', async () => {
  currentBoard = executionBoard('running');
  status.runs = [executionRun()];
  const fetch = mockReads();
  await mount();
  let resolve!: (response: Response) => void;
  const deferred = new Promise<Response>((done) => {
    resolve = done;
  });
  fetch.mockImplementation(async (input: RequestInfo | URL) =>
    String(input) === '/api/v1/execution'
      ? deferred
      : executionResponse([currentBoard]),
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(25_000);
  });
  expect(screen.getByRole('alert')).toHaveTextContent('连接超时');
  expect(screen.getByRole('button', { name: '取消执行' })).toBeDisabled();
  await act(async () => {
    resolve(executionResponse(status));
  });
  expect(screen.getByRole('alert')).toHaveTextContent('连接超时');
  expect(
    screen.queryByRole('region', { name: '执行体进度' }),
  ).not.toBeInTheDocument();
});
