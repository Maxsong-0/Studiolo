import { boardItems } from '../boardModel';
import type { BoardItem } from '../boardModel';
import { ApiError, getJson } from '../sessionHttp';

/** 单次读取黑板明细的时限：超时就作废这次结果，按「状态不可确认」处理。 */
export const BOARD_REQUEST_MS = 10_000;

/**
 * 读一个项目的黑板明细（`/api/v1/board?project=`）并经封闭解码器校验。
 * 「任务与回执」的轮询与拓扑任务详情的按需读取共用这一处：同一条会话闸门、同一个解码器。
 */
export async function fetchBoardItems(
  project: string,
  signal: AbortSignal,
): Promise<BoardItem[]> {
  const value = await getJson<unknown>(
    `/api/v1/board?project=${encodeURIComponent(project)}`,
    signal,
  );
  return boardItems(value, project);
}

/** 黑板读取失败的说法：404 是黑板没启用或入口失效，其余照错误原文。 */
export function boardFailure(error: unknown): string {
  return error instanceof ApiError && error.status === 404
    ? '此工作台尚未启用协作黑板，或黑板入口已失效。'
    : error instanceof Error
      ? error.message
      : '黑板连接不可用。';
}
