import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
} from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ExecutionBoundary } from './ExecutionBoundary';
import { ExecutionLanding } from './ExecutionLanding';
import { installExecutionWriter } from './executionApi';
import { boardItem } from './boardModel';
import { runSummary } from './executionModel';
import { ApiError } from './sessionHttp';
import {
  executionBoard,
  executionHash,
  executionId,
  executionProject,
  executionResponse,
  executionRun,
} from './test/executionFixtures';

const head = 'b'.repeat(40);
const tree = 'c'.repeat(40);
const projects = [{ root: executionProject, name: 'Studiolo' }];
let uninstall: (() => void) | undefined;
let board = patchBoard();
let run = executionRun('candidate_review');
function patchBoard() {
  const value = executionBoard('needs_review');
  value.candidate_receipt = {
    manifest_path: `${executionProject}/03-记忆/manifest.json`,
    manifest_revision: executionHash,
    artifacts: [
      {
        path: `${executionProject}/03-记忆/candidate.patch`,
        revision: executionHash,
        sha256: executionHash,
      },
    ],
  };
  return value;
}
function result() {
  return {
    task_id: executionId(),
    attempt_id: `A-${executionId(101)}`,
    expected_head: head,
    patch_sha256: executionHash,
    after_tree: tree,
    applied: true,
    replayed: false,
    conflicts: [] as string[],
  };
}
function reads() {
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    if (url === '/api/v1/execution')
      return executionResponse({
        configured: true,
        frozen: false,
        runs: [run],
      });
    if (url.startsWith('/api/v1/board?')) return executionResponse([board]);
    if (url.startsWith('/api/v1/board/replay?'))
      return executionResponse({ task: board, events: [], next: null });
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
async function click(element: HTMLElement) {
  await act(async () => {
    fireEvent.click(element);
  });
}
function landingButton() {
  return screen.getByRole('button', { name: '二次认证后落地代码' });
}
function confirm() {
  return screen.getByRole('checkbox', { name: /我已核对目标 HEAD/ });
}
async function approve() {
  fireEvent.change(screen.getByRole('textbox', { name: '目标仓库完整 HEAD' }), {
    target: { value: head },
  });
  await click(confirm());
  await click(landingButton());
}
beforeEach(() => {
  vi.useFakeTimers();
  vi.setSystemTime(Date.UTC(2026, 8, 20, 10));
  board = patchBoard();
  run = executionRun('candidate_review');
});
afterEach(() => {
  cleanup();
  uninstall?.();
  uninstall = undefined;
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

it('requires an explicit HEAD, patch digest and separate human confirmation before one landing POST', async () => {
  reads();
  const writer = vi.fn(async (body: unknown, signal: AbortSignal) => {
    void body;
    void signal;
    return result();
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  expect(landingButton()).toBeDisabled();
  expect(
    screen.getByRole('textbox', { name: '目标仓库完整 HEAD' }),
  ).toHaveValue('');
  expect(
    screen.getByRole('textbox', { name: 'candidate.patch SHA-256' }),
  ).toHaveValue(executionHash);
  fireEvent.change(screen.getByRole('textbox', { name: '目标仓库完整 HEAD' }), {
    target: { value: head },
  });
  expect(landingButton()).toBeDisabled();
  await click(confirm());
  fireEvent.change(
    screen.getByRole('textbox', { name: 'candidate.patch SHA-256' }),
    { target: { value: 'd'.repeat(64) } },
  );
  expect(confirm()).not.toBeChecked();
  await click(confirm());
  await click(landingButton());
  expect(screen.getByRole('alert')).toHaveTextContent('必须与已封存');
  expect(writer).not.toHaveBeenCalled();
  fireEvent.change(
    screen.getByRole('textbox', { name: 'candidate.patch SHA-256' }),
    { target: { value: executionHash } },
  );
  await click(confirm());
  await click(landingButton());
  expect(writer).toHaveBeenCalledTimes(1);
  expect(writer.mock.calls[0]?.[0]).toEqual({
    operation: 'land',
    task_id: executionId(),
    expected_head: head,
    patch_sha256: executionHash,
  });
  expect(
    screen.getByRole('region', { name: '本次代码落地回执' }),
  ).toHaveTextContent('代码已落地，尚待逐项验收');
  expect(screen.getByRole('button', { name: '提交审阅回执' })).toBeDisabled();
});
it('retains landing receipt across authority refresh and never auto reviews or retries success', async () => {
  const fetch = reads();
  const writer = vi.fn(async () => ({ ...result(), replayed: true }));
  uninstall = installExecutionWriter(writer);
  await mount();
  await approve();
  const receipt = screen.getByRole('region', { name: '本次代码落地回执' });
  expect(receipt).toHaveTextContent('已核对此前成功落地，本次未重复应用');
  expect(receipt).toHaveTextContent(head);
  expect(receipt).toHaveTextContent(tree);
  expect(
    fetch.mock.calls.filter(([url]) => url === '/api/v1/execution'),
  ).toHaveLength(2);
  expect(
    fetch.mock.calls.filter(([url]) =>
      String(url).startsWith('/api/v1/board?'),
    ),
  ).toHaveLength(2);
  expect(
    fetch.mock.calls.filter(([url]) =>
      String(url).startsWith('/api/v1/board/replay?'),
    ).length,
  ).toBeGreaterThanOrEqual(2);
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(writer).toHaveBeenCalledTimes(1);
  expect(screen.getByRole('button', { name: '提交审阅回执' })).toBeDisabled();
});
it('conflicts are visible as incomplete landing and cannot become success or automatic review', async () => {
  reads();
  const writer = vi.fn(async () => ({
    ...result(),
    applied: false,
    conflicts: ['src/app.ts', 'src/model.ts'],
  }));
  uninstall = installExecutionWriter(writer);
  await mount();
  await approve();
  expect(screen.getByRole('alert')).toHaveTextContent('代码未完成落地');
  const receipt = screen.getByRole('region', { name: '本次代码落地回执' });
  expect(receipt).toHaveTextContent('不能提交为成功');
  expect(receipt).toHaveTextContent('src/app.ts');
  expect(receipt).toHaveTextContent('src/model.ts');
  expect(
    screen.queryByText('代码已落地，尚待逐项验收。'),
  ).not.toBeInTheDocument();
  expect(screen.getByRole('button', { name: '提交审阅回执' })).toBeDisabled();
  expect(writer).toHaveBeenCalledTimes(1);
});
it.each([
  [new Error('连接中断，落地结果不可确认'), '连接中断，落地结果不可确认'],
  [new ApiError('expired', 401), '登录已失效'],
  [new ApiError('L3 required', 409), '请先在「账户」完成二次认证'],
  [new ApiError('forbidden', 403), '请先在「账户」完成二次认证'],
])('failed landing reports %s and does not retry', async (error, message) => {
  reads();
  const writer = vi.fn(async () => {
    throw error;
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  await approve();
  expect(screen.getByRole('alert')).toHaveTextContent(String(message));
  expect(
    screen.queryByRole('region', { name: '本次代码落地回执' }),
  ).not.toBeInTheDocument();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(writer).toHaveBeenCalledTimes(1);
  expect(landingButton()).toBeDisabled();
});
it('rejects a response for another attempt without displaying landing success', async () => {
  reads();
  uninstall = installExecutionWriter(async () => ({
    ...result(),
    attempt_id: `A-${executionId(202)}`,
  }));
  await mount();
  await approve();
  expect(screen.getByRole('alert')).toHaveTextContent('attempt 已变化');
  expect(
    screen.queryByRole('region', { name: '本次代码落地回执' }),
  ).not.toBeInTheDocument();
});
it('only matching candidate_review attempts with fresh authority may land', async () => {
  const task = boardItem(board);
  const submit = vi.fn(async () => true);
  const candidateRun = runSummary(run);
  const props = {
    task,
    run: candidateRun,
    disabled: false,
    result: null,
    submit,
  };
  const view = render(<ExecutionLanding {...props} />);
  await approve();
  expect(submit).toHaveBeenCalledTimes(1);
  for (const change of [
    { disabled: true },
    { run: { ...candidateRun, state: 'candidate_retained' as const } },
    { run: { ...candidateRun, state: 'running' as const } },
    { run: { ...candidateRun, attempt_id: `A-${executionId(202)}` } },
    { run: undefined },
  ]) {
    view.rerender(<ExecutionLanding {...props} {...change} />);
    expect(landingButton()).toBeDisabled();
  }
  view.rerender(
    <ExecutionLanding {...props} task={{ ...task, candidateReceipt: null }} />,
  );
  expect(
    screen.queryByRole('button', { name: '二次认证后落地代码' }),
  ).not.toBeInTheDocument();
  expect(submit).toHaveBeenCalledTimes(1);
});
it('a failed execution poll clears landing authority even while the board remains available', async () => {
  const fetch = reads();
  await mount();
  fireEvent.change(screen.getByRole('textbox', { name: '目标仓库完整 HEAD' }), {
    target: { value: head },
  });
  await click(confirm());
  expect(landingButton()).toBeEnabled();
  fetch.mockImplementation(async (input: RequestInfo | URL) =>
    String(input) === '/api/v1/execution'
      ? executionResponse({}, 404)
      : String(input).startsWith('/api/v1/board/replay?')
        ? executionResponse({ task: board, events: [], next: null })
        : executionResponse([board]),
  );
  await act(async () => {
    await vi.advanceTimersByTimeAsync(15_000);
  });
  expect(landingButton()).toBeDisabled();
  expect(screen.getByRole('alert')).toHaveTextContent('当前状态不可确认');
});
it('late landing success after the shared command timeout is unknown and never auto retried', async () => {
  reads();
  let resolve!: (value: unknown) => void;
  const waiting = new Promise<unknown>((done) => {
    resolve = done;
  });
  const writer = vi.fn(async (body: unknown, signal: AbortSignal) => {
    void body;
    void signal;
    return waiting;
  });
  uninstall = installExecutionWriter(writer);
  await mount();
  await approve();
  await act(async () => {
    await vi.advanceTimersByTimeAsync(20_000);
  });
  expect(writer.mock.calls[0]?.[1].aborted).toBe(true);
  await act(async () => {
    resolve(result());
  });
  expect(screen.getByRole('alert')).toHaveTextContent('未取得操作结果');
  expect(
    screen.queryByRole('region', { name: '本次代码落地回执' }),
  ).not.toBeInTheDocument();
  expect(writer).toHaveBeenCalledTimes(1);
});
