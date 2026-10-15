import { act, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { App } from '../App';
import { SessionBoundary } from '../SessionBoundary';
import { REVISIT_INTERVAL_MS } from '../sessionClient';
import { defaultApiResponse, jsonResponse } from './fixtures';
import { pair } from './sessionFixtures';

/**
 * 服务端认证入口的真实约束（crates/local-server/src/auth_http.rs `blocking()`）：
 * 会话校验走 `Semaphore::new(2).try_acquire_owned()`，第 3 个同时到达的认证请求
 * 不排队、直接 503「认证结果暂不能确认；操作可能已生效，请先核对状态。」。
 * bootstrap 不做会话校验，不占许可。这里按同样的规则模拟服务端。
 */
const PERMITS = 2;
const UNAVAILABLE = '认证结果暂不能确认；操作可能已生效，请先核对状态。';
const HASH = 'd'.repeat(64);

let inFlight = 0;
let peak = 0;
let rejected: string[] = [];

function meta(version: number) {
  return {
    version,
    kind: 'capability_set',
    subject: 'claude',
    retired: false,
    body_sha256: HASH,
    effective_hash: HASH,
    previous_effective_hash: HASH,
    activated_at: '2026-09-17T00:00:00Z',
    activated_by: 'owner',
    desired_id: null,
    rollback_of: null,
    expansions: [],
  };
}

function route(path: string, init?: RequestInit): Response {
  switch (path) {
    case '/api/v1/bootstrap':
      return jsonResponse({ authentication: 'session' });
    case '/api/v1/session':
      // 生产会话带主人范围：只有这时才会挂上按主人范围启用的组件（文件操作、草稿等）。
      return jsonResponse({ ...pair, owner_scope: 'e'.repeat(64) });
    case '/api/v1/auth/state':
      return jsonResponse({ factors: null });
    case '/api/v1/policy':
      return jsonResponse({
        effective_hash: HASH,
        entries: [],
        global_revoked: false,
        pending_desired: 0,
        revocations: [],
        l3_available: true,
      });
    case '/api/v1/policy/desired':
      return jsonResponse({ desired: [], newer_open: {} });
    case '/api/v1/policy/versions':
      return jsonResponse({ versions: [meta(1)] });
    case '/api/v1/policy/versions/1':
      return jsonResponse({ version: { meta: meta(1), body: '第一版正文' } });
    default:
      return init?.method === 'POST'
        ? jsonResponse({ error: '未知' }, 404)
        : defaultApiResponse(path);
  }
}

function server(input: RequestInfo | URL, init?: RequestInit) {
  const path = String(input);
  const gated = path !== '/api/v1/bootstrap';
  if (gated) {
    if (inFlight >= PERMITS) {
      rejected.push(path);
      return Promise.resolve(jsonResponse({ error: UNAVAILABLE }, 503));
    }
    inFlight += 1;
    peak = Math.max(peak, inFlight);
  }
  return new Promise<Response>((resolve) => {
    setTimeout(() => {
      if (gated) inFlight -= 1;
      resolve(route(path, init));
    }, 15);
  });
}

function boundary() {
  return (
    <SessionBoundary>{(account) => <App account={account} />}</SessionBoundary>
  );
}

/**
 * 真实的切走再回来。一天内回来不再核对（ADR-022）；要走到「重新核对→整页重挂」
 * 这条路，先把时钟拨过一天。
 */
async function leaveAndReturn({ afterADay = false } = {}) {
  if (afterADay)
    vi.spyOn(Date, 'now').mockReturnValue(
      Date.now() + REVISIT_INTERVAL_MS + 1000,
    );
  for (const state of ['hidden', 'visible'] as const) {
    Object.defineProperty(document, 'visibilityState', {
      configurable: true,
      get: () => state,
    });
    await act(async () => {
      document.dispatchEvent(new Event('visibilitychange'));
    });
  }
}

beforeEach(() => {
  inFlight = 0;
  peak = 0;
  rejected = [];
  vi.stubGlobal('fetch', vi.fn(server));
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('认证请求不超过服务端的并发许可', () => {
  it('刷新后直接落在策略页（带版本 diff），不会报「认证结果暂不能确认」', async () => {
    window.location.hash = '#view=policy&item=1';
    render(boundary());
    expect(await screen.findByText('第一版正文')).toBeInTheDocument();
    await waitFor(() => expect(inFlight).toBe(0));
    expect(screen.queryByText(/认证结果暂不能确认/)).not.toBeInTheDocument();
    expect(rejected).toEqual([]);
    expect(peak).toBeLessThanOrEqual(PERMITS);
  });

  it('超过一天后在策略页切回（重新核对、整页重挂），同样不超并发', async () => {
    window.location.hash = '#view=policy&item=1';
    render(boundary());
    expect(await screen.findByText('第一版正文')).toBeInTheDocument();
    await waitFor(() => expect(inFlight).toBe(0));
    peak = 0;
    rejected = [];

    await leaveAndReturn({ afterADay: true });
    // 确实走了重新核对（不是一天内的「什么都不做」）。
    await waitFor(() => expect(peak).toBeGreaterThan(0));

    expect(await screen.findByText('第一版正文')).toBeInTheDocument();
    await waitFor(() => expect(inFlight).toBe(0));
    expect(screen.queryByText(/认证结果暂不能确认/)).not.toBeInTheDocument();
    expect(rejected).toEqual([]);
    expect(peak).toBeLessThanOrEqual(PERMITS);
  });
});
