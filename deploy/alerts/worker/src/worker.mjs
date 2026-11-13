import { createHash, timingSafeEqual } from 'node:crypto';
import { deliver } from './delivery.mjs';

const NODES = Object.freeze([
  Object.freeze({ node: 'n108', token: 'NODE_108_TOKEN', code: 'A108' }),
  Object.freeze({ node: 'n169', token: 'NODE_169_TOKEN', code: 'A169' }),
]);
const STALE_SECONDS = 180;
const COOLDOWN_SECONDS = 600;
const LEASE_SECONDS = 45;
const unavailable = () => new Error('heartbeat_unavailable');
const logFailure = () => console.error('{"event":"heartbeat_check_failed"}');

function checkTime(now) {
  if (!Number.isSafeInteger(now) || now < 0 || now > Number.MAX_SAFE_INTEGER - LEASE_SECONDS) {
    throw unavailable();
  }
}

function matchesToken(provided, expected) {
  const bytes = new TextEncoder();
  if (typeof expected !== 'string' || !expected || bytes.encode(expected).length > 1024) {
    return false;
  }
  if (!provided || bytes.encode(provided).length > 1024) return false;
  const digest = (token) => createHash('sha256').update(token).digest();
  return timingSafeEqual(digest(provided), digest(expected));
}

async function emptyBody(request) {
  if (request.body === null) return true;
  const reader = request.body.getReader();
  let timer;
  try {
    // Workers 会给 Content-Length: 0 的 POST 一个空流；最多四个空块、一秒。
    const deadline = new Promise((_, reject) => { timer = setTimeout(() => reject(unavailable()), 1000); });
    for (let index = 0; index < 4; index++) {
      const chunk = await Promise.race([reader.read(), deadline]);
      if (chunk.value?.byteLength) return false;
      if (chunk.done) return true;
    }
    return false;
  } finally {
    clearTimeout(timer);
    void reader.cancel().catch(() => {});
  }
}

export async function handleHeartbeat(request, env, nowSeconds) {
  try {
    const url = new URL(request.url);
    const node = NODES.find((item) => url.pathname === `/heartbeat/${item.node}`);
    if (request.method !== 'POST' || !node || url.href.includes('?')) {
      return new Response('not_found', { status: 404 });
    }
    const authorization = request.headers.get('authorization') ?? '';
    const token = authorization.startsWith('Bearer ') ? authorization.slice(7) : '';
    if (!matchesToken(token, env[node.token])) return new Response('unauthorized', { status: 401 });
    if (!await emptyBody(request)) return new Response('not_found', { status: 404 });
    checkTime(nowSeconds);
    const updated = await env.DB.prepare(`UPDATE heartbeats
      SET last_seen = MAX(last_seen, ?1), last_sent = NULL, lease_until = 0
      WHERE node = ?2 AND ?1 >= last_seen RETURNING node`)
      .bind(nowSeconds, node.node).first();
    if (!updated) {
      // 回拨时保留较新的心跳和故障状态；缺少固定种子行则不能假装心跳已入库。
      const existing = await env.DB.prepare('SELECT last_seen FROM heartbeats WHERE node = ?1')
        .bind(node.node).first();
      if (!existing || existing.last_seen <= nowSeconds) throw unavailable();
    }
    return new Response(null, { status: 204 });
  } catch {
    logFailure();
    return new Response('unavailable', { status: 503 });
  }
}

async function release(env, node, lastSeen, lease) {
  await env.DB.prepare(`UPDATE heartbeats SET lease_until = 0
    WHERE node = ?1 AND last_seen = ?2 AND lease_until = ?3`)
    .bind(node, lastSeen, lease).run();
}

export async function checkHeartbeats(env, nowSeconds, send = deliver) {
  checkTime(nowSeconds);
  let failed = false;
  for (const { node, code } of NODES) {
    const lease = nowSeconds + LEASE_SECONDS;
    let claimed;
    try {
      // 条件更新同时检查阈值、冷却和租约，并发 cron 只有一个能领到这一情节。
      claimed = await env.DB.prepare(`UPDATE heartbeats SET lease_until = ?1
        WHERE node = ?2 AND last_seen <= ?3 AND lease_until <= ?4
          AND (last_sent IS NULL OR last_sent <= ?5) RETURNING last_seen`)
        .bind(lease, node, nowSeconds - STALE_SECONDS, nowSeconds, nowSeconds - COOLDOWN_SECONDS)
        .first();
      if (!claimed) {
        const existing = await env.DB.prepare('SELECT last_seen FROM heartbeats WHERE node = ?1').bind(node).first();
        if (!existing) throw unavailable();
        continue;
      }
      await send(env, code);
      // 新心跳会改变 last_seen 或清掉租约；旧发送的迟到回执不能覆盖新情节。
      await env.DB.prepare(`UPDATE heartbeats SET last_sent = ?1, lease_until = 0
        WHERE node = ?2 AND last_seen = ?3 AND lease_until = ?4`)
        .bind(nowSeconds, node, claimed.last_seen, lease).run();
    } catch {
      failed = true;
      logFailure();
      if (claimed) {
        try { await release(env, node, claimed.last_seen, lease); }
        catch { logFailure(); }
      }
    }
  }
  // 两个节点都检查完再让调度事件失败，避免第一台的故障掩盖第二台。
  if (failed) throw unavailable();
}

export default {
  async fetch(request, env) {
    return await handleHeartbeat(request, env, Math.floor(Date.now() / 1000));
  },
  async scheduled(controller, env) {
    await checkHeartbeats(env, Math.floor(controller.scheduledTime / 1000));
  },
};
