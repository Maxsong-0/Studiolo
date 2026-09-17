import { expect, it, vi } from 'vitest';
import { SessionClient } from '../sessionClient';
import { installCanonicalRestorer, replaceCanonical } from '../canonicalApi';
import { readHistoryFile, readHistoryPage } from '../historyApi';
import { installSessionReader } from '../sessionHttp';
import { canonical, committed, ownerScope } from './canonicalFixtures';
import { historyEntry, historyFile, historyPage } from './historyFixtures';
import { csrf, mockSessions, pair, sessionResponse } from './sessionFixtures';
import { jsonResponse } from './fixtures';

it('恢复只走受控 restore + CSRF，历史正文永不变成 replace 或传入请求体', async () => {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (path, init) => {
    if (path === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (path === '/api/v1/mutations')
      return jsonResponse(committed(JSON.parse(String(init?.body))));
    return sessionResponse(path, init);
  });
  const client = new SessionClient();
  await client.check();
  const uninstall = installCanonicalRestorer(client.restore);
  const request = {
    kind: 'restore' as const,
    vault_uuid: canonical.vault_uuid,
    path: canonical.path,
    expected_revision: canonical.revision,
    source_revision: historyFile.revision,
    idempotency_key: 'fixture-restore-1',
    content: historyFile.content,
  };
  try {
    await replaceCanonical(request, new AbortController().signal);
    const call = fetchMock.mock.calls.find(
      ([path]) => path === '/api/v1/mutations',
    )!;
    expect(call[1]).toMatchObject({
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'X-CSRF-Token': csrf },
    });
    const body = JSON.parse(String(call[1]?.body));
    expect(body).toEqual({
      kind: 'restore',
      vault_uuid: request.vault_uuid,
      path: request.path,
      expected_revision: request.expected_revision,
      source_revision: request.source_revision,
      idempotency_key: request.idempotency_key,
    });
    expect(body).not.toHaveProperty('content');
  } finally {
    uninstall();
    client.stop();
  }
});
it('历史 GET 只投影库/路径/游标/版本，受控 reader 不泄漏原文参数', async () => {
  const read = vi.fn(async (url: string) =>
    url.startsWith('/api/v1/history/file?') ? historyFile : historyPage,
  );
  const uninstall = installSessionReader(read);
  try {
    await readHistoryPage(canonical, null, new AbortController().signal);
    await readHistoryFile(
      canonical,
      historyEntry,
      new AbortController().signal,
    );
    const list = new URL(read.mock.calls[0][0], 'https://fixture.invalid');
    expect([...list.searchParams.keys()]).toEqual([
      'vault_uuid',
      'path',
      'limit',
    ]);
    expect(list.searchParams.get('limit')).toBe('20');
    expect([
      ...new URL(read.mock.calls[1][0], list).searchParams.keys(),
    ]).toEqual(['vault_uuid', 'path', 'revision']);
    expect(read.mock.calls.map(([url]) => url).join('')).not.toContain(
      'content',
    );
  } finally {
    uninstall();
  }
});
