import {
  act,
  cleanup,
  fireEvent,
  render,
  screen,
  within,
} from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ExecutionBoundary } from './ExecutionBoundary';
import { boardItem } from './boardModel';
import { COMMAND_TARGETS } from './commands/focusTarget';
import { installExecutionWriter } from './executionApi';
import { runSummary } from './executionModel';
import {
  EXECUTION_STEPS,
  needsAttention,
  runStep,
  taskStep,
} from './executionView';
import { ApiError } from './sessionHttp';
import {
  executionBoard,
  executionEvent,
  executionHash,
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

it('draws execution progress as nine segments with progressbar values and the current step in words', async () => {
  currentBoard = executionBoard('needs_review');
  status.runs = [executionRun('candidate_review')];
  mockReads();
  await mount();
  const text = '第 8 步，共 9 步。当前：落地与审阅';
  const run = screen.getByRole('progressbar', {
    name: `执行进度 ${executionId()}`,
  });
  expect(run).toHaveAttribute('aria-valuemin', '0');
  expect(run).toHaveAttribute('aria-valuemax', '9');
  expect(run).toHaveAttribute('aria-valuenow', '8');
  expect(run).toHaveAttribute('aria-valuetext', text);
  expect(
    Array.from(run.querySelectorAll('[data-state]'), (segment) =>
      segment.getAttribute('data-state'),
    ),
  ).toEqual([...Array<string>(7).fill('done'), 'current', 'todo']);
  // 第 8 段「落地与审阅」等你：当前段标红（规范第 1 节：红色只给等你或出问题）
  expect(run.parentElement).toHaveClass('execution-steps--attention');
  const task = screen.getByRole('progressbar', {
    name: '执行进度：Runner 契约任务',
  });
  expect(task).toHaveAttribute('aria-valuenow', '8');
  expect(task).toHaveAttribute('aria-valuemax', '9');
  expect(screen.getAllByText(text)).toHaveLength(2);
  // 这条候选没有 candidate.patch：不用落地，只等逐项审阅
  expect(screen.getByText('候选已封存，等你逐项审阅。')).toBeInTheDocument();
  // 顶部「执行」标题栏标出当前任务在执行服务里的 run id
  expect(
    screen.getByRole('heading', { name: '执行', level: 2 }).parentElement,
  ).toHaveTextContent(`run A-${executionId(101)}`);
});
it('a run that cannot be placed on the nine steps shows — with the reason instead of a guessed step', async () => {
  currentBoard = executionBoard('running');
  status.runs = [executionRun('stopped')];
  mockReads();
  await mount();
  expect(screen.queryAllByRole('progressbar')).toHaveLength(0);
  expect(screen.getAllByText('九步进度 —')).toHaveLength(2);
  expect(screen.getAllByText(/接口没有给出它停在哪一段/)).toHaveLength(2);
  expect(screen.getByText('已停止')).toBeInTheDocument();
});
it('a queued task sits on step 1 and the header says why it has no run id', async () => {
  mockReads();
  await mount();
  const task = screen.getByRole('progressbar', {
    name: '执行进度：Runner 契约任务',
  });
  expect(task).toHaveAttribute('aria-valuenow', '1');
  expect(task).toHaveAttribute(
    'aria-valuetext',
    '第 1 步，共 9 步。当前：启动',
  );
  expect(
    screen.getByRole('heading', { name: '执行', level: 2 }).parentElement,
  ).toHaveTextContent('run —这条任务在执行服务里没有执行记录。');
});
it('the review sheet is the ⌘K focus target for 逐项审阅候选', async () => {
  currentBoard = executionBoard('needs_review');
  status.runs = [executionRun('candidate_review')];
  mockReads();
  await mount();
  const form = screen.getByRole('form', { name: '逐项审阅候选' });
  expect(form).toHaveAttribute(
    'data-command-target',
    COMMAND_TARGETS.executionReview,
  );
  expect(form).toHaveAttribute('tabindex', '-1');
  expect(
    within(form).getByRole('textbox', { name: '验收回执 JSON' }),
  ).toBeInTheDocument();
  act(() => form.focus());
  expect(form).toHaveFocus();
});

describe('执行九步的阶段表', () => {
  const task = (value: ReturnType<typeof executionBoard>) =>
    boardItem(value, executionProject);
  const run = (state: string, phase = 'working') =>
    runSummary({
      ...executionRun(state),
      progress: { tool_calls: 5, edits: 2, phase },
    });
  const where = (value: ReturnType<typeof runStep>) =>
    value.known ? [value.step, value.name, value.halted] : ['—'];

  it('only waiting-on-you steps (1 and 8) and halted runs are marked red', () => {
    expect(needsAttention(runStep(run('running', 'working')))).toBe(false);
    expect(needsAttention(runStep(run('running', 'wrapup')))).toBe(false);
    expect(needsAttention(runStep(run('candidate_review')))).toBe(true);
    expect(needsAttention(runStep(run('running', 'exhausted')))).toBe(true);
    expect(needsAttention({ known: false, reason: '无' })).toBe(false);
    expect(
      needsAttention({
        known: true,
        step: 9,
        name: '结案',
        finished: true,
        halted: false,
        note: '',
      }),
    ).toBe(false);
  });

  it('maps every run state and budget phase onto the nine steps', () => {
    expect(EXECUTION_STEPS).toHaveLength(9);
    expect(where(runStep(run('running', 'working')))).toEqual([
      4,
      '工作',
      false,
    ]);
    expect(where(runStep(run('running', 'pacing')))).toEqual([
      5,
      '节奏检查',
      false,
    ]);
    expect(where(runStep(run('running', 'wrapup')))).toEqual([
      6,
      '准备收尾',
      false,
    ]);
    expect(where(runStep(run('running', 'exhausted')))).toEqual([
      6,
      '准备收尾',
      true,
    ]);
    expect(where(runStep(run('candidate_retained')))).toEqual([
      7,
      '提交候选',
      true,
    ]);
    expect(where(runStep(run('candidate_review')))).toEqual([
      8,
      '落地与审阅',
      false,
    ]);
    expect(where(runStep(run('failed', 'pacing')))).toEqual([
      5,
      '节奏检查',
      true,
    ]);
    expect(where(runStep(run('stopped')))).toEqual(['—']);
  });
  it('places tasks by board status, trusts only the same attempt and never guesses', () => {
    expect(
      taskStep(task(executionBoard('queued')), undefined, null, true),
    ).toMatchObject({ known: true, step: 1, name: '启动', finished: false });
    expect(
      taskStep(
        task(executionBoard('running')),
        run('running', 'pacing'),
        null,
        true,
      ),
    ).toMatchObject({ known: true, step: 5 });
    expect(
      taskStep(
        task(executionBoard('running')),
        { ...run('running', 'pacing'), attempt_id: `A-${executionId(202)}` },
        null,
        true,
      ),
    ).toMatchObject({ known: false });
    expect(
      taskStep(task(executionBoard('running')), undefined, null, false),
    ).toEqual({
      known: false,
      reason: '执行服务状态不可确认，停在哪一段不可确认。',
    });
    expect(
      taskStep(task(executionBoard('cancelled')), undefined, null, true).known,
    ).toBe(false);
    // 黑板已封存但执行服务说候选只是留存：审阅锁着，停在第 7 段而不是「等你审阅」
    const sealed = task(executionBoard('needs_review'));
    expect(
      taskStep(sealed, run('candidate_retained'), null, true),
    ).toMatchObject({ known: true, step: 7, halted: true });
    expect(taskStep(sealed, undefined, null, true)).toMatchObject({
      known: true,
      step: 8,
      note: '候选已封存，等你逐项审阅。',
    });
    const closed = executionBoard('succeeded');
    closed.board.receipt = executionReceipt();
    expect(taskStep(task(closed), undefined, null, true)).toMatchObject({
      known: true,
      step: 9,
      finished: true,
      note: '回执：已完成。',
    });
  });
  it('a code candidate waits for landing first; an applied landing receipt leaves only the review', () => {
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
    const code = task(value);
    const review = run('candidate_review');
    const landing = {
      task_id: executionId(),
      attempt_id: `A-${executionId(101)}`,
      patch_sha256: executionHash,
      expected_head: 'b'.repeat(40),
      after_tree: 'c'.repeat(40),
      applied: true,
      replayed: false,
      conflicts: [] as string[],
    };
    expect(taskStep(code, review, null, true)).toMatchObject({
      step: 8,
      note: '代码候选：先核对 HEAD 并落地，再逐项审阅。',
    });
    expect(taskStep(code, review, landing, true)).toMatchObject({
      step: 8,
      note: '代码已落地，等你逐项审阅。',
    });
    expect(
      taskStep(
        code,
        review,
        { ...landing, applied: false, conflicts: ['src/app.ts'] },
        true,
      ),
    ).toMatchObject({ note: '代码候选：先核对 HEAD 并落地，再逐项审阅。' });
  });
});
