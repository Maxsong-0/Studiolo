// 只发送固定代号；成功仅表示推送服务接受，不表示手机已响或用户已读。
const LIMIT = 4096;
const TIMEOUT_MS = 8000;
const failure = () => new Error('delivery_failed');

function secret(env, name) {
  const value = env[name];
  if (typeof value !== 'string' || value.trim().length === 0) throw failure();
  return value;
}

function alertLink(env) {
  if (env.ALERT_LINK === undefined || env.ALERT_LINK === '') return undefined;
  if (typeof env.ALERT_LINK !== 'string') throw failure();
  const url = new URL(env.ALERT_LINK);
  if (url.protocol !== 'https:' || url.username || url.password || url.search || url.hash
      || env.ALERT_LINK.includes('?') || env.ALERT_LINK.includes('#')) throw failure();
  return url.href;
}

async function responseJson(response, signal) {
  if (!response.ok || !response.body) {
    await response.body?.cancel().catch(() => undefined);
    throw failure();
  }
  const reader = response.body.getReader();
  const chunks = [];
  let length = 0;
  try {
    while (true) {
      signal.throwIfAborted();
      const { done, value } = await reader.read();
      if (done) break;
      length += value.byteLength;
      if (length > LIMIT) throw failure();
      chunks.push(value);
    }
    const bytes = new Uint8Array(length);
    let offset = 0;
    for (const chunk of chunks) { bytes.set(chunk, offset); offset += chunk.byteLength; }
    return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(bytes));
  } catch {
    await reader.cancel().catch(() => undefined);
    throw failure();
  } finally {
    reader.releaseLock();
  }
}

async function post(url, body, contentType, fetcher) {
  const signal = AbortSignal.timeout(TIMEOUT_MS);
  const response = await fetcher(url, {
    method: 'POST', redirect: 'error', signal,
    headers: { 'content-type': contentType }, body,
  });
  return responseJson(response, signal);
}

export async function deliver(env, code, fetcher = fetch) {
  if (code !== 'A108' && code !== 'A169') throw failure();
  const message = `Needs attention: code ${code}`;
  try {
    const form = new URLSearchParams({
      token: secret(env, 'PUSHOVER_TOKEN'), user: secret(env, 'PUSHOVER_USER'),
      message, priority: '2', retry: '60', expire: '3000',
    });
    const link = alertLink(env);
    if (link) form.set('url', link);
    const result = await post('https://api.pushover.net/1/messages.json', form.toString(),
      'application/x-www-form-urlencoded', fetcher);
    if (result?.status !== 1 || typeof result.receipt !== 'string' || !result.receipt.trim()) {
      throw failure();
    }
    return 'pushover';
  } catch {
    // 主通道未确认接受才走备用；原错误、响应与配置值一律不进入日志或返回值。
  }
  try {
    const result = await post('https://api.day.app/push', JSON.stringify({
      device_key: secret(env, 'BARK_DEVICE_KEY'), body: message,
      level: 'critical', volume: 10, call: 1, isArchive: 0,
    }), 'application/json', fetcher);
    if (result?.code !== 200) throw failure();
    return 'bark';
  } catch {
    throw failure();
  }
}
