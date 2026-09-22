export class ApiError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly details?: unknown,
  ) {
    super(message);
    this.name = 'ApiError';
  }
}

export function cancelled(): never {
  throw new DOMException('请求已取消。', 'AbortError');
}

/**
 * 服务端认证入口同一时刻只校验两个会话（`auth_http.rs` 的 `blocking()` 用
 * `Semaphore::new(2).try_acquire_owned()`）：第三个同时到达的请求不排队，直接 503
 * 「认证结果暂不能确认」。所有认证请求都经过这里，于是在这里排队：在途最多两个，
 * 其余按到达顺序等；排队中被取消的请求不会发出去。bootstrap 不校验会话，不占名额。
 */
export const AUTH_SLOTS = 2;
const UNGATED = '/api/v1/bootstrap';
let slotsInUse = 0;
const waiting: (() => void)[] = [];

function handOver() {
  const next = waiting.shift();
  // 名额直接交给下一个排队者，在途数不变；没人排队才归还。
  if (next) next();
  else slotsInUse -= 1;
}

function releaser() {
  let released = false;
  return () => {
    if (released) return;
    released = true;
    handOver();
  };
}

/** 有空名额就同步拿到（请求照旧在同一拍发出，不多等一个微任务）；满了才排队。 */
function takeSlot(signal: AbortSignal): (() => void) | Promise<() => void> {
  if (signal.aborted) cancelled();
  if (slotsInUse < AUTH_SLOTS) {
    slotsInUse += 1;
    return releaser();
  }
  return new Promise((resolve, reject) => {
    const wake = () => {
      signal.removeEventListener('abort', leave);
      resolve(releaser());
    };
    const leave = () => {
      const index = waiting.indexOf(wake);
      if (index >= 0) waiting.splice(index, 1);
      reject(new DOMException('请求已取消。', 'AbortError'));
    };
    waiting.push(wake);
    signal.addEventListener('abort', leave, { once: true });
  });
}

export async function requestJson(
  path: string,
  signal: AbortSignal,
  body?: unknown,
  csrf?: string,
  success: 'json' | 'json-200' | 'empty-204' = 'json',
): Promise<unknown> {
  const headers: Record<string, string> = {
    Accept: 'application/json',
    'X-Studiolo-Request': '1',
  };
  if (body !== undefined) headers['Content-Type'] = 'application/json';
  if (csrf) headers['X-CSRF-Token'] = csrf;
  const slot = path === UNGATED ? null : takeSlot(signal);
  const release = slot instanceof Promise ? await slot : slot;
  // 服务端的名额在会话校验结束时就释放，早于响应头；所以拿到响应头（或被取消）即归还，
  // 不必等正文读完。取消时立刻归还，免得一个不理会取消的请求把队列卡死。
  const onAbort = () => release?.();
  signal.addEventListener('abort', onAbort, { once: true });
  let response: Response;
  try {
    response = await fetch(path, {
      method: body === undefined ? 'GET' : 'POST',
      signal,
      credentials: 'same-origin',
      cache: 'no-store',
      redirect: 'error',
      headers,
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    });
  } finally {
    signal.removeEventListener('abort', onAbort);
    release?.();
  }
  if (signal.aborted) cancelled();
  if (response.status === 401) throw new ApiError('登录已失效。', 401);
  if (!response.ok) {
    const value: unknown = await response.json().catch(() => null);
    if (signal.aborted) cancelled();
    const message =
      value && typeof value === 'object' && 'error' in value
        ? String(value.error)
        : `请求失败（${response.status}）`;
    throw new ApiError(message, response.status, value);
  }
  if (success === 'empty-204') {
    if (response.status !== 204) throw new Error('安全操作的响应格式无效。');
    return null;
  }
  if (success === 'json-200' && response.status !== 200)
    throw new Error('安全操作的响应格式无效。');
  const value: unknown = await response.json();
  if (signal.aborted) cancelled();
  return value;
}

export function linkedSignal(signals: AbortSignal[]) {
  const controller = new AbortController();
  const abort = () => controller.abort();
  for (const signal of signals) {
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) controller.abort();
  }
  return {
    signal: controller.signal,
    dispose: () => {
      for (const signal of signals) signal.removeEventListener('abort', abort);
    },
  };
}

type Reader = (path: string, signal: AbortSignal) => Promise<unknown>;
let activeReader: Reader | null = null;

// Only the real entry installs the gate; App remains independently renderable.
export function installSessionReader(reader: Reader) {
  activeReader = reader;
  return () => {
    if (activeReader === reader) activeReader = null;
  };
}

export async function getJson<T>(
  path: string,
  signal: AbortSignal,
): Promise<T> {
  return (await (activeReader ?? requestJson)(path, signal)) as T;
}
