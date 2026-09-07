import { ApiError, requestJson } from './sessionHttp';
import { authentication } from './sessionModel';

export async function discoverSession(signal: AbortSignal) {
  const mode = authentication(await requestJson('/api/v1/bootstrap', signal));
  if (mode !== 'session') return { phase: 'legacy' as const };
  try {
    return {
      phase: 'ready' as const,
      value: await requestJson('/api/v1/session', signal),
    };
  } catch (error) {
    // 只有已明确session模式后的自身401可进入显式登录；bootstrap错误不降级。
    if (error instanceof ApiError && error.status === 401)
      return { phase: 'signed-out' as const };
    throw error;
  }
}
