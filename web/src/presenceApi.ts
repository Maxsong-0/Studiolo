import { ApiError, getJson } from './sessionHttp';
import { parsePresence, type Presence } from './presenceModel';

/**
 * 读「在做什么」状态（驾驶舱「本机会话」）。走与其它内容接口相同的会话闸门，响应一律经封闭解码器。
 * 404 表示本机没装黑板，用 {@link presenceDisabled} 认；其余失败都以错误抛出，绝不当成「没有会话」。
 */
export async function fetchPresence(signal: AbortSignal): Promise<Presence> {
  return parsePresence(await getJson('/api/v1/board/presence', signal));
}

export function presenceDisabled(error: unknown): boolean {
  return error instanceof ApiError && error.status === 404;
}
