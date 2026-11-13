import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { DatabaseSync } from 'node:sqlite';
import test from 'node:test';
import worker, { handleHeartbeat, checkHeartbeats } from '../src/worker.mjs';
import { deliver } from '../src/delivery.mjs';

const migration = readFileSync(new URL('../migrations/0001-heartbeats.sql', import.meta.url), 'utf8');
// 适配接口而不改写 SQL：所有条件、返回行和约束均由真实 SQLite 执行。
function fixture(t, now = 1000) {
  const sql = new DatabaseSync(':memory:');
  sql.exec(migration);
  if (now !== null) sql.prepare('UPDATE heartbeats SET last_seen = ?').run(now);
  t.after(() => sql.close());
  const DB = { prepare(text) {
    const statement = sql.prepare(text);
    let params = {};
    return {
      bind(...values) { params = Object.fromEntries(values.map((value, i) => [`?${i + 1}`, value])); return this; },
      async first() { return statement.get(params) ?? null; },
      async run() { return { success: true, meta: statement.run(params) }; },
    };
  } };
  return { sql, env: { DB, NODE_108_TOKEN: 'test-only-108', NODE_169_TOKEN: 'test-only-169' },
    row: (node) => ({ ...sql.prepare('SELECT * FROM heartbeats WHERE node = ?').get(node) }) };
}
const request = (node = 'n108', token = 'test-only-108', options = {}) => new Request(
  `https://heartbeat.example/heartbeat/${node}`, {
    method: 'POST', headers: { authorization: `Bearer ${token}` }, ...options,
  });
function deferred() { let resolve; const promise = new Promise((done) => { resolve = done; }); return { promise, resolve }; }
const json = (value, status = 200) => new Response(JSON.stringify(value), { status });
const secrets = () => ({ PUSHOVER_TOKEN: 'test-push-token', PUSHOVER_USER: 'test-user', BARK_DEVICE_KEY: 'test-device' });
const accepted = () => json({ status: 1, receipt: 'test-receipt' });

test('迁移仅建两节点，首次 180 秒宽限', async (t) => {
  const { sql, env, row } = fixture(t, null);
  const at = row('n108').last_seen;
  assert.deepEqual(sql.prepare('SELECT node FROM heartbeats ORDER BY node').all().map((r) => r.node), ['n108', 'n169']);
  assert.throws(() => sql.prepare('INSERT INTO heartbeats(node,last_seen) VALUES (?,?)').run('n999', at));
  const calls = [];
  await checkHeartbeats(env, at + 179, async (_, code) => calls.push(code));
  assert.deepEqual(calls, []);
  await checkHeartbeats(env, at + 180, async (_, code) => calls.push(code));
  assert.deepEqual(calls, ['A108', 'A169']);
});

test('两节点独立鉴权，正确心跳清除冷却和租约', async (t) => {
  const { env, sql, row } = fixture(t);
  assert.equal((await handleHeartbeat(request('n169'), env, 1100)).status, 401);
  assert.equal((await handleHeartbeat(request('n108', 'test-only-169'), env, 1100)).status, 401);
  sql.exec("UPDATE heartbeats SET last_sent=1050,lease_until=1200 WHERE node='n108'");
  assert.equal((await handleHeartbeat(request(), env, 1100)).status, 204);
  assert.deepEqual(row('n108'), { node: 'n108', last_seen: 1100, last_sent: null, lease_until: 0 });
  assert.equal(row('n169').last_seen, 1000);
  assert.equal((await handleHeartbeat(request('n169', 'test-only-169'), env, 1110)).status, 204);
});

test('错误方法、路径、query、正文统一拒绝且不写库', async (t) => {
  const { env, row } = fixture(t);
  const invalid = [request('n999'), request('n108/'), request('n108?x=1'), request('n108?'),
    request('n108', 'test-only-108', { method: 'GET' }),
    request('n108', 'test-only-108', { body: 'private-note' })];
  for (const req of invalid) assert.equal((await handleHeartbeat(req, env, 1100)).status, 404);
  assert.equal(row('n108').last_seen, 1000);
  assert.equal((await handleHeartbeat(request('n108', 'test-only-108', { body: '' }), env, 1100)).status, 204);
});

test('cron 缺少固定节点行时失败，仍检查另一台', async (t) => {
  const { env, sql } = fixture(t); const calls = [];
  t.mock.method(console, 'error', () => undefined);
  sql.exec("DELETE FROM heartbeats WHERE node='n108'");
  await assert.rejects(checkHeartbeats(env, 1180, async (_env, code) => { calls.push(code); }), /heartbeat_unavailable/);
  assert.deepEqual(calls, ['A169']);
});

test('空、缺少、错误格式、超长 token 拒绝，1024 字节边界接受', async (t) => {
  const { env } = fixture(t);
  for (const token of ['', 'wrong', 'x'.repeat(1025)]) {
    assert.equal((await handleHeartbeat(request('n108', token), env, 1100)).status, 401);
  }
  assert.equal((await handleHeartbeat(request('n108', '', { headers: {} }), env, 1100)).status, 401);
  assert.equal((await handleHeartbeat(request('n108', '', { headers: { authorization: 'Basic test-only-108' } }), env, 1100)).status, 401);
  for (const token of ['', undefined, 'x'.repeat(1025), 'é'.repeat(513)]) {
    assert.equal((await handleHeartbeat(request('n108', token ?? ''), { ...env, NODE_108_TOKEN: token }, 1100)).status, 401);
  }
  for (const token of ['x'.repeat(1024), 'é'.repeat(512)]) {
    assert.equal((await handleHeartbeat(request('n108', token), { ...env, NODE_108_TOKEN: token }, 1100)).status, 204);
  }
});

test('fetch 等待落库，回拨不倒退或清冷却，缺种子行和数据库故障返回固定 503', async (t) => {
  const { env, sql, row } = fixture(t);
  t.mock.method(console, 'error', () => undefined);
  const gate = deferred(); const prepare = env.DB.prepare.bind(env.DB);
  env.DB.prepare = (text) => { const query = prepare(text); const first = query.first.bind(query);
    query.first = async () => { await gate.promise; return first(); }; return query; };
  let finished = false;
  const pending = worker.fetch(request(), env).then((result) => { finished = true; return result; });
  await Promise.resolve(); assert.equal(finished, false); gate.resolve();
  assert.equal((await pending).status, 204);
  sql.exec("UPDATE heartbeats SET last_seen=1200,last_sent=1190,lease_until=1245 WHERE node='n108'");
  assert.equal((await handleHeartbeat(request(), env, 1199)).status, 204);
  assert.deepEqual(row('n108'), { node: 'n108', last_seen: 1200, last_sent: 1190, lease_until: 1245 });
  sql.exec("DELETE FROM heartbeats WHERE node='n108'");
  assert.equal((await handleHeartbeat(request(), env, 1300)).status, 503);
  const bad = { ...env, DB: { prepare() { throw new Error('SECRET SQL'); } } };
  const response = await handleHeartbeat(request(), bad, 1300);
  assert.equal(response.status, 503); assert.equal(await response.text(), 'unavailable');
});

test('单节点故障独立触发，成功后 600 秒冷却，恢复后新情节立即重新计时', async (t) => {
  const { env, row } = fixture(t); const calls = [];
  const send = async (_, code) => calls.push(code);
  await handleHeartbeat(request('n169', 'test-only-169'), env, 1170);
  await checkHeartbeats(env, 1179, send); assert.deepEqual(calls, []);
  await checkHeartbeats(env, 1180, send); assert.deepEqual(calls, ['A108']);
  await handleHeartbeat(request('n169', 'test-only-169'), env, 1770);
  await checkHeartbeats(env, 1779, send); assert.equal(calls.length, 1);
  await checkHeartbeats(env, 1780, send); assert.deepEqual(calls, ['A108', 'A108']);
  await handleHeartbeat(request(), env, 1781);
  assert.equal(row('n108').last_sent, null);
  await handleHeartbeat(request('n169', 'test-only-169'), env, 1960);
  await checkHeartbeats(env, 1961, send); assert.deepEqual(calls, ['A108', 'A108', 'A108']);
});

test('并发 cron 只领取一次，过期租约可接续', async (t) => {
  const { env, sql, row } = fixture(t); const entered = deferred(); const gate = deferred(); const calls = [];
  await handleHeartbeat(request('n169', 'test-only-169'), env, 1180);
  const first = checkHeartbeats(env, 1180, async (_, code) => { calls.push(code); entered.resolve(); await gate.promise; });
  await entered.promise;
  assert.equal(row('n108').lease_until, 1225);
  await checkHeartbeats(env, 1180, async (_, code) => calls.push(code));
  assert.deepEqual(calls, ['A108']); gate.resolve(); await first;
  sql.exec("UPDATE heartbeats SET last_sent=NULL,lease_until=1225 WHERE node='n108'");
  await checkHeartbeats(env, 1224, async (_, code) => calls.push(code)); assert.equal(calls.length, 1);
  await checkHeartbeats(env, 1225, async (_, code) => calls.push(code)); assert.equal(calls.length, 2);
});

test('失败不记冷却，两节点都尝试；下轮重试成功', async (t) => {
  const { env, row } = fixture(t); const calls = [];
  const logs = []; t.mock.method(console, 'error', (line) => logs.push(line));
  await assert.rejects(checkHeartbeats(env, 1180, async (_, code) => {
    calls.push(code); if (code === 'A108') throw new Error('SECRET response');
  }), { message: 'heartbeat_unavailable' });
  assert.deepEqual(calls, ['A108', 'A169']);
  assert.equal(row('n108').last_sent, null); assert.equal(row('n108').lease_until, 0);
  assert.equal(row('n169').last_sent, 1180);
  await checkHeartbeats(env, 1240, async (_, code) => calls.push(code));
  assert.deepEqual(calls, ['A108', 'A169', 'A108']);
  assert.deepEqual(logs, ['{"event":"heartbeat_check_failed"}']);
});

test('旧发送成功或失败迟到都不能覆盖恢复后的新心跳', async (t) => {
  t.mock.method(console, 'error', () => undefined);
  for (const rejects of [false, true]) {
    const { env, row } = fixture(t); const entered = deferred(); const gate = deferred();
    await handleHeartbeat(request('n169', 'test-only-169'), env, 1180);
    const old = checkHeartbeats(env, 1180, async () => { entered.resolve(); await gate.promise; if (rejects) throw Error('private'); });
    const outcome = old.then(() => null, (error) => error);
    await entered.promise; await handleHeartbeat(request(), env, 1181);
    if (!rejects) {
      await handleHeartbeat(request('n169', 'test-only-169'), env, 1361);
      await checkHeartbeats(env, 1361, async () => 'pushover');
    }
    gate.resolve();
    const result = await outcome;
    assert.equal(result?.message ?? null, rejects ? 'heartbeat_unavailable' : null);
    assert.deepEqual(row('n108'), { node: 'n108', last_seen: 1181, last_sent: rejects ? null : 1361, lease_until: 0 });
  }
});

test('D1 一个节点失败仍检查第二个，scheduled 等待检查并暴露固定失败', async (t) => {
  const { env } = fixture(t); const calls = []; t.mock.method(console, 'error', () => undefined);
  const prepare = env.DB.prepare.bind(env.DB);
  env.DB.prepare = (text) => { const query = prepare(text); const bind = query.bind.bind(query);
    query.bind = (...values) => { if (values.includes('n108')) throw Error('SECRET SQL'); return bind(...values); }; return query; };
  await assert.rejects(checkHeartbeats(env, 1180, async (_, code) => calls.push(code)), { message: 'heartbeat_unavailable' });
  assert.deepEqual(calls, ['A169']);
  const clean = fixture(t);
  t.mock.method(globalThis, 'fetch', async () => { assert.fail('测试不得外发'); });
  await assert.rejects(worker.scheduled({ scheduledTime: 1180000 }, clean.env), { message: 'heartbeat_unavailable' });
  await worker.scheduled({ scheduledTime: 999000 }, clean.env);
});

test('主路紧急参数、固定目的地与仅代号正文；可选 HTTPS 链接', async (t) => {
  const timeouts = []; t.mock.method(AbortSignal, 'timeout', (ms) => { timeouts.push(ms); return new AbortController().signal; });
  const env = { ...secrets(), ALERT_LINK: 'https://studiolo.example/', PRIVATE_NOTE: 'do-not-send' };
  const calls = [];
  assert.equal(await deliver(env, 'A108', async (url, options) => { calls.push({ url, options }); return accepted(); }), 'pushover');
  assert.deepEqual(timeouts, [8000]); assert.equal(calls.length, 1);
  assert.equal(calls[0].url, 'https://api.pushover.net/1/messages.json');
  assert.equal(calls[0].options.redirect, 'error'); assert.equal(calls[0].options.method, 'POST');
  assert.deepEqual(Object.fromEntries(new URLSearchParams(calls[0].options.body)), {
    token: 'test-push-token', user: 'test-user', message: 'Needs attention: code A108',
    priority: '2', retry: '60', expire: '3000', url: 'https://studiolo.example/',
  });
  assert.ok(!calls[0].options.body.includes('do-not-send'));
});

test('两路各自收到八秒超时信号，超时只返回固定错误', async (t) => {
  const timeouts = [];
  t.mock.method(AbortSignal, 'timeout', (ms) => {
    timeouts.push(ms); return AbortSignal.abort(new DOMException('private-timeout', 'TimeoutError'));
  });
  await assert.rejects(deliver(secrets(), 'A108', async (_, options) => {
    options.signal.throwIfAborted(); assert.fail('已超时不应继续');
  }), { message: 'delivery_failed' });
  assert.deepEqual(timeouts, [8000, 8000]);
});

// 全部错误均为本地注入，无网络请求，也不等待真实的八秒超时。
const failures = [
  ['HTTP', () => json({ private: 'not logged' }, 503)],
  ['重定向', () => new Response(null, { status: 302, headers: { location: 'https://private.example/' } })],
  ['超时', () => { throw new DOMException('SECRET timeout', 'TimeoutError'); }],
  ['超长', () => new Response('x'.repeat(4097))],
  ['UTF8 字节超长', () => new Response('é'.repeat(3000))],
  ['畸形JSON', () => new Response('{SECRET')],
  ['空响应', () => new Response(null)],
  ['业务拒绝', () => json({ status: 0, code: 400 })],
];
for (const [name, fail] of failures) {
  test(`主路${name}转备用，备用也失败则固定错误`, async () => {
    const calls = [];
    assert.equal(await deliver(secrets(), 'A169', async (url, options) => {
      calls.push({ url, options }); return calls.length === 1 ? fail() : json({ code: 200 });
    }), 'bark');
    assert.equal(calls[1].url, 'https://api.day.app/push'); assert.equal(calls[1].options.redirect, 'error');
    assert.deepEqual(JSON.parse(calls[1].options.body), { device_key: 'test-device', body: 'Needs attention: code A169',
      level: 'critical', volume: 10, call: 1, isArchive: 0 });
    await assert.rejects(deliver(secrets(), 'A169', async () => fail()), { message: 'delivery_failed' });
  });
}

test('Pushover 必须有成功状态与非空紧急回执，Bark 必须数值 200', async () => {
  for (const value of [{ status: 1 }, { status: 1, receipt: '' }, { status: 1, receipt: ' ' },
    { status: '1', receipt: 'r' }, { status: 1, receipt: 3 }, null]) {
    let calls = 0;
    assert.equal(await deliver(secrets(), 'A108', async () => ++calls === 1 ? json(value) : json({ code: 200 })), 'bark');
    assert.equal(calls, 2);
  }
  for (const code of [0, '200', null]) await assert.rejects(deliver(secrets(), 'A108', async () => json({ code })), { message: 'delivery_failed' });
});

test('无配置明确失败，非法代号不请求，非法链接不外发', async () => {
  const forbidden = async () => assert.fail('不应外发');
  await assert.rejects(deliver({}, 'A108', forbidden), { message: 'delivery_failed' });
  await assert.rejects(deliver(secrets(), 'PRIVATE-HOST', forbidden), { message: 'delivery_failed' });
  for (const link of ['http://private.example', 'https://user:password@example.com',
    'https://example.com/?secret', 'https://example.com/#secret', 'https://example.com/?', 'https://example.com/#']) {
    const calls = [];
    assert.equal(await deliver({ ...secrets(), ALERT_LINK: link }, 'A108', async (url, options) => {
      calls.push(url); assert.ok(!options.body.includes(link)); return json({ code: 200 });
    }), 'bark');
    assert.deepEqual(calls, ['https://api.day.app/push']);
  }
});

test('响应流按累计字节限额，4096 内正常接受，超限取消读取', async () => {
  let cancelled = false;
  const content = JSON.stringify({ status: 1, receipt: 'r', padding: ' '.repeat(4000) });
  assert.ok(new TextEncoder().encode(content).length <= 4096);
  assert.equal(await deliver(secrets(), 'A108', async () => new Response(content)), 'pushover');
  let calls = 0;
  assert.equal(await deliver(secrets(), 'A108', async () => {
    if (++calls === 2) return json({ code: 200 });
    return new Response(new ReadableStream({ start(controller) {
      controller.enqueue(new Uint8Array(2048)); controller.enqueue(new Uint8Array(2049));
    }, cancel() { cancelled = true; } }));
  }), 'bark');
  assert.equal(cancelled, true);
});
