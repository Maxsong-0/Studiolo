import { vi } from 'vitest';
import type { FactorState } from '../factorModel';
import { SessionClient } from '../sessionClient';
import { jsonResponse } from './fixtures';
import { sessionResponse } from './sessionFixtures';

const observed = Date.now();
export const recent = {
  verified_at_ms: observed - 1_000,
  expires_at_ms: observed + 299_000,
};
export const factorSnapshot: FactorState = {
  observed_at_ms: observed,
  recent_authentication: null,
  recovery_candidate_available: true,
  pending: false,
};
export const readyFactors = {
  ...factorSnapshot,
  recent_authentication: recent,
};
export const codes = Array.from(
  { length: 10 },
  (_, index) =>
    `r1.0000000000000001.0${index}.${(index + 1).toString(16).padStart(32, '0')}`,
);
export function factorResponse(
  input: RequestInfo | URL,
  init?: RequestInit,
): Response {
  switch (String(input)) {
    case '/api/v1/auth/state':
      return jsonResponse({ factors: readyFactors });
    case '/api/v1/auth/step-up':
      return jsonResponse(recent);
    case '/api/v1/auth/recovery-codes':
      return jsonResponse({ codes });
    case '/api/v1/auth/factor/recover':
      return new Response(null, { status: 204 });
    default:
      return sessionResponse(input, init);
  }
}
export function mockFactors() {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockImplementation(async (input, init) => factorResponse(input, init));
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
}
export async function factorClient() {
  const client = new SessionClient();
  await client.check();
  await client.factorState(new AbortController().signal);
  return client;
}
