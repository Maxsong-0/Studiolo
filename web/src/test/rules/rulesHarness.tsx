import { StrictMode } from 'react';
import { act, render } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { vi } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { SessionBoundary } from '../../SessionBoundary';
import { RulesView } from '../../views/RulesView';

export interface SentRequest {
  method: string;
  path: string;
  body: unknown;
}

/**
 * 用开发假数据的应答器当后端：会话、策略平面的读写与二次认证都走真实入口（SessionBoundary 装配的
 * 受控写者与因子运行时），每个请求原样记下供断言。时钟用真实时间，会话寿命与近期认证窗口都按此刻算。
 */
export function serveFixtures() {
  const api = createFixtureApi();
  const sent: SentRequest[] = [];
  const fetchMock = vi.fn(
    async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input);
      const method = init?.method ?? 'GET';
      const headers = Object.fromEntries(
        Object.entries((init?.headers ?? {}) as Record<string, string>).map(
          ([name, value]) => [name.toLowerCase(), value],
        ),
      );
      const body: unknown =
        init?.body === undefined ? undefined : JSON.parse(String(init.body));
      sent.push({ method, path, body });
      const outcome = api.handle({ method, url: path, headers, body });
      return outcome.status === 204
        ? new Response(null, { status: 204 })
        : new Response(JSON.stringify(outcome.body ?? null), {
            status: outcome.status,
            headers: { 'Content-Type': 'application/json' },
          });
    },
  );
  vi.stubGlobal('fetch', fetchMock);
  return {
    api,
    sent,
    posts: (path: string) =>
      sent.filter(
        (request) => request.method === 'POST' && request.path === path,
      ),
    /** 直接向假数据后端读一份 JSON（不经界面、不记进 sent）。 */
    read: (path: string) =>
      api.handle({ method: 'GET', url: path, headers: {} }).body,
  };
}

/** 带位置打开规则视图（包在会话边界里，与真实入口同一装配）。 */
export async function mountRules(hash: string) {
  window.location.hash = hash;
  const user = userEvent.setup();
  await act(async () => {
    render(
      <StrictMode>
        <SessionBoundary>
          {(account) => (
            <>
              {account}
              <RulesView />
            </>
          )}
        </SessionBoundary>
      </StrictMode>,
    );
  });
  return user;
}
