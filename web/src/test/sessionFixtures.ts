import { vi } from 'vitest';
import { defaultApiResponse, jsonResponse } from './fixtures';
import type { SessionInfo } from '../sessionModel';

export const currentSession: SessionInfo = {
  id: 'v2-0000000000000001',
  state: 'active',
  created_at_ms: 1_700_000_000_000,
  last_seen_at_ms: 1_700_000_100_000,
  absolute_expires_at_ms: Date.now() + 3_600_000,
};
export const otherSession: SessionInfo = {
  ...currentSession,
  id: 'v2-0000000000000002',
};
export const csrf = `c2.0000000000000001.${'a'.repeat(64)}`;
export const pair = { session: currentSession, csrf_token: csrf };
export function sessionResponse(
  input: RequestInfo | URL,
  init?: RequestInit,
): Response {
  switch (String(input)) {
    case '/api/v1/bootstrap':
      return jsonResponse({ authentication: 'session' });
    case '/api/v1/session':
      return jsonResponse(pair);
    case '/api/v1/auth/state':
      return jsonResponse({ factors: null });
    case '/api/v1/sessions':
      return init?.method === 'POST'
        ? jsonResponse(pair, 201)
        : jsonResponse({ sessions: [currentSession, otherSession] });
    case '/api/v1/sessions/revoke':
      return jsonResponse({ revoked: 1 });
    default:
      return defaultApiResponse(input);
  }
}
export function mockSessions() {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(async (input, init) => sessionResponse(input, init));
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}
export function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((finish) => {
    resolve = finish;
  });
  return { promise, resolve };
}
