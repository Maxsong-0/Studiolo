import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { Miniflare, convertV4MiniflareOptions } from 'miniflare';

test('实际 Workers 运行时使用 D1 落心跳且拒绝跨节点凭据', async () => {
  const runtime = new Miniflare(convertV4MiniflareOptions({
    modules: ['worker.mjs', 'delivery.mjs'].map((name) => ({
      type: 'ESModule', path: fileURLToPath(new URL('../src/' + name, import.meta.url)),
    })),
    compatibilityDate: '2026-11-13',
    compatibilityFlags: ['nodejs_compat'],
    d1Databases: ['DB'],
    bindings: { NODE_108_TOKEN: 'local-test-one', NODE_169_TOKEN: 'local-test-two' },
    outboundService: () => { throw new Error('测试不允许真实对外请求'); },
  }));
  try {
    const database = await runtime.getD1Database('DB');
    const schema = await readFile(new URL('../migrations/0001-heartbeats.sql', import.meta.url), 'utf8');
    for (const statement of schema.split(';').filter((part) => part.trim())) {
      await database.prepare(statement).run();
    }
    await database.prepare('UPDATE heartbeats SET last_seen = 1').run();
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
  } finally {
    await runtime.dispose();
  }
});
