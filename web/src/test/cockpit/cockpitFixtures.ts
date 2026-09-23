/**
 * 驾驶舱测试的输入：直接取开发假数据（web/dev）的应答器，喂给真实解码器，
 * 与 devFixtures.test.ts 同一时刻，免得测试里另起一份与假数据脱节的样例。
 */
import { createFixtureApi } from '../../../dev/fixtureApi';
import type { FixtureResponse } from '../../../dev/fixtureApi';
import {
  parseBoardOverview,
  type BoardOverview,
} from '../../boardOverviewModel';
import { parseHealth, type HealthReport } from '../../healthApi';
import type { Workbench } from '../../types';
import { validateWorkbench } from '../../validate';

/** 今天本地 20:31：假数据的事件正好落在当天 08:00–20:00。 */
export const NOW = new Date(2026, 8, 22, 20, 31, 0, 0).getTime();

export const OVERVIEW_URL = '/api/v1/board/overview?window_hours=24';

export type FixtureApi = ReturnType<typeof createFixtureApi>;

export function fixtureApi(now = NOW): FixtureApi {
  return createFixtureApi({ now: () => now });
}

function get(api: FixtureApi, url: string): FixtureResponse {
  return api.handle({
    method: 'GET',
    url,
    headers: { 'x-studiolo-request': '1' },
  });
}

export function read(api: FixtureApi, url: string): unknown {
  const outcome = get(api, url);
  if (outcome.status !== 200)
    throw new Error(`假数据 ${url} 应答 ${outcome.status}`);
  return outcome.body;
}

export interface FixtureData {
  api: FixtureApi;
  workbench: Workbench;
  overview: BoardOverview;
  health: HealthReport;
}

export function fixtureData(now = NOW): FixtureData {
  const api = fixtureApi(now);
  return {
    api,
    workbench: validateWorkbench(read(api, '/api/workbench')),
    overview: parseBoardOverview(read(api, OVERVIEW_URL), 24),
    health: parseHealth(read(api, '/api/v1/health')),
  };
}

export function toResponse(outcome: FixtureResponse): Response {
  return outcome.body === undefined
    ? new Response(null, { status: outcome.status })
    : new Response(JSON.stringify(outcome.body), {
        status: outcome.status,
        headers: { 'Content-Type': 'application/json' },
      });
}

/**
 * 把假数据应答器接成 fetch：`overrides` 按路径前缀改写个别接口
 * （比如让黑板汇总回 404、让本机健康失败），其余照假数据应答。
 */
export function fixtureFetch(
  api: FixtureApi,
  overrides: Record<string, () => Response | Promise<Response>> = {},
): (input: RequestInfo | URL) => Promise<Response> {
  return async (input) => {
    const url = String(input);
    const override = Object.entries(overrides).find(([prefix]) =>
      url.startsWith(prefix),
    );
    return override ? override[1]() : toResponse(get(api, url));
  };
}
