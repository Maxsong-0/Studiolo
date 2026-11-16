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
  const response = await requestResponse(path, signal, {
    method: body === undefined ? 'GET' : 'POST',
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

async function requestResponse(
  path: string,
  signal: AbortSignal,
  options: RequestInit,
): Promise<Response> {
  const slot = path === UNGATED ? null : takeSlot(signal);
  const release = slot instanceof Promise ? await slot : slot;
  // 服务端的名额在会话校验结束时就释放，早于响应头；所以拿到响应头（或被取消）即归还，
  // 不必等正文读完。取消时立刻归还，免得一个不理会取消的请求把队列卡死。
  const onAbort = () => release?.();
  signal.addEventListener('abort', onAbort, { once: true });
  let response: Response;
  try {
    if (signal.aborted) cancelled();
    response = await fetch(path, {
      ...options,
      signal,
      credentials: 'same-origin',
      cache: 'no-store',
      redirect: 'error',
    });
  } finally {
    signal.removeEventListener('abort', onAbort);
    release?.();
  }
  return response;
}

export const MAX_ATTACHMENT_BYTES = 32 * 1024 * 1024;

/** 长度在读体前、流中、完成时各核一次，避免损坏响应或伪造长度无限占用内存。 */
export async function requestBlob(
  path: string,
  signal: AbortSignal,
  expectedBytes: number,
): Promise<Blob> {
  if (
    !Number.isSafeInteger(expectedBytes) ||
    expectedBytes < 0 ||
    expectedBytes > MAX_ATTACHMENT_BYTES
  )
    throw new Error('附件超出网页读取上限，请在同步副本中打开。');
  const response = await requestResponse(path, signal, {
    method: 'GET',
    headers: { Accept: '*/*', 'X-Studiolo-Request': '1' },
  });
  if (signal.aborted) cancelled();
  if (response.status === 401) throw new ApiError('登录已失效。', 401);
  if (response.status !== 200) {
    await response.body?.cancel();
    throw new ApiError(
      response.status === 409
        ? '附件已变化，请刷新工作台后重试。'
        : `附件读取失败（${response.status}）。`,
      response.status,
    );
  }
  if (response.headers.get('content-length') !== String(expectedBytes)) {
    await response.body?.cancel();
    throw new Error('附件大小与当前快照不符，请刷新工作台。');
  }
  const reader = response.body?.getReader();
  const chunks: Uint8Array<ArrayBuffer>[] = [];
  let received = 0;
  try {
    if (reader) {
      while (true) {
        const { done, value } = await reader.read();
        if (signal.aborted) cancelled();
        if (done) break;
        received += value.byteLength;
        if (received > expectedBytes) throw new Error('附件响应超出声明大小。');
        chunks.push(new Uint8Array(value));
      }
    }
    if (signal.aborted) cancelled();
    if (received !== expectedBytes)
      throw new Error('附件内容不完整，请重新读取。');
    return new Blob(chunks, {
      type: response.headers.get('content-type') ?? 'application/octet-stream',
    });
  } finally {
    if (reader) {
      await reader.cancel().catch(() => undefined);
      reader.releaseLock();
    }
  }
}

type BlobReader = typeof requestBlob;
let activeBlobReader: BlobReader | null = null;
export function installSessionBlobReader(reader: BlobReader) {
  activeBlobReader = reader;
  return () => {
    if (activeBlobReader === reader) activeBlobReader = null;
  };
}
export function getBlob(
  path: string,
  signal: AbortSignal,
  expectedBytes: number,
) {
  return (activeBlobReader ?? requestBlob)(path, signal, expectedBytes);
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
