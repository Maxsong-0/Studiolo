import { ApiError, getJson } from './sessionHttp';
import {
  BOARD_OVERVIEW_MAX_WINDOW_HOURS,
  parseBoardOverview,
} from './boardOverviewModel';
import type { BoardOverview } from './boardOverviewModel';

/**
 * 读跨项目黑板汇总（驾驶舱与协作拓扑共用）。走与其它内容接口相同的会话闸门
 * （`getJson` → `requestJson`，认证请求在途最多两个），响应一律经封闭解码器。
 * 401/404/429/503 都以 `ApiError` 抛出，绝不当成空黑板；404 表示本机未启用黑板，
 * 用 {@link boardOverviewDisabled} 识别后显示空态。
 */
export async function fetchBoardOverview(
  windowHours: number,
  signal: AbortSignal,
): Promise<BoardOverview> {
  if (
    !Number.isSafeInteger(windowHours) ||
    windowHours < 1 ||
    windowHours > BOARD_OVERVIEW_MAX_WINDOW_HOURS
  )
    throw new RangeError('黑板汇总时间窗必须是 1–168 的整数小时。');
  const query = new URLSearchParams({ window_hours: String(windowHours) });
  return parseBoardOverview(
    await getJson(`/api/v1/board/overview?${query}`, signal),
    windowHours,
  );
}

/** 服务端未装配黑板时汇总接口回 404；其余失败（含 401）都不是「未启用」。 */
export function boardOverviewDisabled(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404;
}
