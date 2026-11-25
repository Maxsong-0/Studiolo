import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';

// 对外请求一律由 outbound 接住，没有真实网络；D1 两行心跳都停在第 1 秒。
async function withRuntime(outbound, use) {
  const runtime = new Miniflare(convertV4MiniflareOptions({
    modules: ['worker.mjs', 'delivery.mjs'].map((name) => ({
      type: 'ESModule', path: fileURLToPath(new URL('../src/' + name, import.meta.url)),
    })),
    compatibilityDate: '2026-11-13',
    compatibilityFlags: ['nodejs_compat'],
    d1Databases: ['DB'],
    bindings: {
      NODE_108_TOKEN: 'local-test-one', NODE_169_TOKEN: 'local-test-two',
      PUSHOVER_TOKEN: 'test-push-token', PUSHOVER_USER: 'test-user', BARK_DEVICE_KEY: 'test-device',
    },
    outboundService: outbound,
  }));
  try {
    const database = await runtime.getD1Database('DB');
    const schema = await readFile(new URL('../migrations/0001-heartbeats.sql', import.meta.url), 'utf8');
    for (const statement of schema.split(';').filter((part) => part.trim())) {
      await database.prepare(statement).run();
    }
    await database.prepare('UPDATE heartbeats SET last_seen = 1').run();
    await use(runtime, database);
  } finally {
    await runtime.dispose();
  }
}

const row = (database, node) =>
  database.prepare('SELECT last_seen, last_sent, lease_until FROM heartbeats WHERE node = ?1').bind(node).first();
const reply = (value, status = 200, headers = {}) => new Response(JSON.stringify(value), { status, headers });
const PUSHOVER = 'https://api.pushover.net/1/messages.json';
const BARK = 'https://api.day.app/push';
const accepted = { [PUSHOVER]: { status: 1, receipt: 'r' }, [BARK]: { code: 200 } };

// 真运行时里跑一次 cron：只有 n108 停了 180 秒，答复由 answer(url) 决定。
async function cron(answer, check) {
  const calls = [];
  await withRuntime(async (request) => {
    calls.push({ url: request.url, method: request.method, body: await request.text() });
    return answer(request.url);
  }, async (runtime, database) => {
    await database.prepare("UPDATE heartbeats SET last_seen = 1170 WHERE node = 'n169'").run();
    const result = await (await runtime.getWorker()).scheduled({
      scheduledTime: new Date(1_180_000), cron: '* * * * *',
    });
    await check({ calls, outcome: result.outcome, n108: await row(database, 'n108'), n169: await row(database, 'n169') });
  });
}

test('实际 Workers 运行时：心跳停了就真发出 Pushover，成功才记冷却', async () => {
  await cron((url) => reply(accepted[url]), ({ calls, outcome, n108, n169 }) => {
    assert.deepEqual(calls.map(({ url, method }) => [url, method]), [[PUSHOVER, 'POST']]);
    assert.deepEqual(Object.fromEntries(new URLSearchParams(calls[0].body)), {
      token: 'test-push-token', user: 'test-user', message: 'Needs attention: code A108',
      priority: '2', retry: '60', expire: '3000',
    });
    assert.equal(outcome, 'ok');
    assert.deepEqual(n108, { last_seen: 1, last_sent: 1180, lease_until: 0 });
    assert.deepEqual(n169, { last_seen: 1170, last_sent: null, lease_until: 0 });
  });
});

test('实际 Workers 运行时：Pushover 拒收转 Bark', async () => {
  await cron((url) => url === PUSHOVER ? reply({ status: 0 }, 400) : reply(accepted[url]), ({ calls, outcome, n108 }) => {
    assert.deepEqual(calls.map(({ url, method }) => [url, method]), [[PUSHOVER, 'POST'], [BARK, 'POST']]);
    assert.deepEqual(JSON.parse(calls[1].body), { device_key: 'test-device', body: 'Needs attention: code A108',
      level: 'critical', volume: 10, call: 1, isArchive: 0 });
    assert.equal(outcome, 'ok');
    assert.equal(n108.last_sent, 1180);
  });
});

// 两路都答跳转、正文写成「已接受」：不许跟过去，也不许把它当成功。
for (const status of [302, 307]) {
  test(`实际 Workers 运行时：两路都答 ${status} 跳转，不跟过去，按失败下轮再发`, async () => {
    await cron((url) => reply(accepted[url] ?? accepted[PUSHOVER], status, { location: 'https://elsewhere.invalid/' }),
      ({ calls, outcome, n108 }) => {
        assert.deepEqual(calls.map(({ url }) => url), [PUSHOVER, BARK]);
        assert.equal(outcome, 'exception');
        assert.deepEqual(n108, { last_seen: 1, last_sent: null, lease_until: 0 });
      });
  });
}

test('实际 Workers 运行时使用 D1 落心跳且拒绝跨节点凭据', async () => {
  await withRuntime(() => { throw new Error('测试不允许真实对外请求'); }, async (runtime, database) => {
    const response = await runtime.dispatchFetch('https://heartbeat.invalid/heartbeat/n108', {
      method: 'POST', headers: { Authorization: 'Bearer local-test-one' },
    });
    assert.equal(response.status, 204);
    assert.ok((await database.prepare("SELECT last_seen FROM heartbeats WHERE node = 'n108'").first()).last_seen > 1);
    const refused = await runtime.dispatchFetch('https://heartbeat.invalid/heartbeat/n169', {
      method: 'POST', headers: { Authorization: 'Bearer local-test-one' },
    });
    assert.equal(refused.status, 401);
    assert.equal((await database.prepare("SELECT last_seen FROM heartbeats WHERE node = 'n169'").first()).last_seen, 1);
    const body = await runtime.dispatchFetch('https://heartbeat.invalid/heartbeat/n108', {
      method: 'POST', headers: { Authorization: 'Bearer local-test-one' }, body: 'not-empty',
    });
    assert.equal(body.status, 404);
  });
});
