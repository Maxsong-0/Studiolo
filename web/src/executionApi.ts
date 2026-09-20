import { ApiError, getJson } from './sessionHttp';
import { boardItem, boardItems } from './boardModel';
import {
  executionStatus,
  landingResult,
  replayPage,
  runSummary,
} from './executionModel';
import type { ExecutionCommand, LandingResult } from './executionModel';

type Writer = (body: unknown, signal: AbortSignal) => Promise<unknown>;
let writer: Writer | null = null;
export function installExecutionWriter(next: Writer) {
  writer = next;
  return () => {
    if (writer === next) writer = null;
  };
}
export async function readExecution(signal: AbortSignal) {
  return executionStatus(await getJson('/api/v1/execution', signal));
}
export async function readExecutionTasks(project: string, signal: AbortSignal) {
  return boardItems(
    await getJson(`/api/v1/board?${new URLSearchParams({ project })}`, signal),
    project,
  );
}
export async function readReplay(
  taskId: string,
  project: string,
  after: string | null,
  signal: AbortSignal,
) {
  const query = new URLSearchParams({ task_id: taskId });
  if (after !== null) query.set('after', after);
  return replayPage(
    await getJson(`/api/v1/board/replay?${query}`, signal),
    taskId,
    project,
    after,
  );
}
/** Only SessionBoundary can supply the authenticated writer; no fetch fallback. */
export async function commandExecution(
  command: ExecutionCommand,
  signal: AbortSignal,
): Promise<LandingResult | undefined> {
  if (!writer) throw new ApiError('尚未装配受控会话写入，请重新登录。', 403);
  const value = await writer(command, signal);
  if (command.operation === 'land') {
    return landingResult(value, command);
  } else if (command.operation === 'start') {
    const run = runSummary(value);
    if (run.task_id !== command.task_id || run.adapter !== command.agent_type)
      throw new Error('启动响应归属无效，请刷新核对。');
  } else if (command.operation === 'review') {
    if (boardItem(value).id !== command.task_id)
      throw new Error('审阅响应归属无效，请刷新核对。');
  } else {
    const field = command.operation === 'freeze' ? 'frozen' : 'ok';
    const expected = command.operation === 'freeze' ? command.frozen : true;
    if (
      !value ||
      typeof value !== 'object' ||
      Array.isArray(value) ||
      Object.keys(value).length !== 1 ||
      (value as Record<string, unknown>)[field] !== expected
    )
      throw new Error('操作响应无效，请刷新核对。');
  }
  return undefined;
}
export function executionFailure(
  error: unknown,
  command?: ExecutionCommand,
): string {
  const message =
    error instanceof Error ? error.message : '连接失败，当前状态不可确认。';
  if (error instanceof ApiError && error.status === 401)
    return '登录已失效，请重新登录。';
  const highRisk =
    command?.operation === 'land' ||
    command?.operation === 'review' ||
    (command?.operation === 'freeze' && !command.frozen);
  if (
    highRisk &&
    error instanceof ApiError &&
    (error.status === 403 || /L3|reauth|二次认证|One.time.Code/i.test(message))
  )
    return '本次未完成：请先在「账户」完成二次认证（One-time Code），然后重新提交。不会自动重试。';
  if (error instanceof ApiError && error.status === 404)
    return '本入口未接入执行服务，当前状态不可确认。';
  return message;
}
