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
  const response = await fetch(path, {
    method: body === undefined ? 'GET' : 'POST',
    signal,
    credentials: 'same-origin',
    cache: 'no-store',
    redirect: 'error',
    headers,
    ...(body === undefined ? {} : { body: JSON.stringify(body) }),
  });
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
