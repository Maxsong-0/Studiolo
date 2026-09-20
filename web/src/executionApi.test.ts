import { expect, it, vi } from 'vitest';
import {
  commandExecution,
  installExecutionWriter,
  readReplay,
} from './executionApi';
import {
  executionBoard,
  executionHash,
  executionId,
  executionProject,
  executionResponse,
} from './test/executionFixtures';
import { SessionClient } from './sessionClient';
import {
  csrf,
  mockSessions,
  pair,
  sessionResponse,
} from './test/sessionFixtures';
import { ownerScope } from './test/canonicalFixtures';
import { jsonResponse } from './test/fixtures';

it('has no unauthenticated write fallback', async () => {
  const fetch = vi.fn();
  vi.stubGlobal('fetch', fetch);
  await expect(
    commandExecution(
      { operation: 'freeze', frozen: true },
      new AbortController().signal,
    ),
  ).rejects.toMatchObject({ status: 403 });
  expect(fetch).not.toHaveBeenCalled();
});
it('execution uses the session writer, owner scope and CSRF on its fixed endpoint', async () => {
  const fetch = mockSessions();
  fetch.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (input === '/api/v1/execution/command')
      return jsonResponse({ frozen: true });
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  await client.check();
  const uninstall = installExecutionWriter(client.executionCommand);
  try {
    await commandExecution(
      { operation: 'freeze', frozen: true },
      new AbortController().signal,
    );
    const call = fetch.mock.calls.find(
      ([path]) => path === '/api/v1/execution/command',
    );
    expect(call?.[1]).toMatchObject({
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'X-CSRF-Token': csrf },
    });
    expect(JSON.parse(String(call?.[1]?.body))).toEqual({
      operation: 'freeze',
      frozen: true,
    });
  } finally {
    uninstall();
    client.stop();
  }
});
it('landing goes through the same session writer and CSRF, returns only a validated bound receipt', async () => {
  const command = {
    operation: 'land' as const,
    task_id: executionId(),
    expected_head: 'b'.repeat(40),
    patch_sha256: executionHash,
  };
  const result = {
    task_id: executionId(),
    attempt_id: `A-${executionId(101)}`,
    expected_head: command.expected_head,
    patch_sha256: executionHash,
    after_tree: 'c'.repeat(40),
    applied: true,
    replayed: false,
    conflicts: [],
  };
  const fetch = mockSessions();
  fetch.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (input === '/api/v1/execution/command') return jsonResponse(result);
    return sessionResponse(input, init);
  });
  const client = new SessionClient();
  await client.check();
  const uninstall = installExecutionWriter(client.executionCommand);
  try {
    await expect(
      commandExecution(command, new AbortController().signal),
    ).resolves.toEqual(result);
    const calls = fetch.mock.calls.filter(
      ([path]) => path === '/api/v1/execution/command',
    );
    expect(calls).toHaveLength(1);
    expect(calls[0]?.[1]).toMatchObject({
      method: 'POST',
      credentials: 'same-origin',
      headers: { 'X-CSRF-Token': csrf },
    });
    expect(JSON.parse(String(calls[0]?.[1]?.body))).toEqual(command);
  } finally {
    uninstall();
    client.stop();
  }
});
it('read-only session cannot start an execution', async () => {
  const fetch = mockSessions();
  const client = new SessionClient();
  await client.check();
  await expect(
    client.executionCommand(
      { operation: 'freeze', frozen: true },
      new AbortController().signal,
    ),
  ).rejects.toMatchObject({ status: 403 });
  expect(
    fetch.mock.calls.some(([path]) => path === '/api/v1/execution/command'),
  ).toBe(false);
  client.stop();
});
it('replay sends the exact string cursor', async () => {
  const fetch = vi.fn(async (input: RequestInfo | URL) => {
    void input;
    return executionResponse({
      task: executionBoard(),
      events: [],
      next: null,
    });
  });
  vi.stubGlobal('fetch', fetch);
  await readReplay(
    executionId(),
    executionProject,
    '9007199254740993',
    new AbortController().signal,
  );
  expect(fetch.mock.calls[0]?.[0]).toBe(
    `/api/v1/board/replay?task_id=${executionId()}&after=9007199254740993`,
  );
});
