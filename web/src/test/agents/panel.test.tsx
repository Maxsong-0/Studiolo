import { StrictMode } from 'react';
import { render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, it, vi } from 'vitest';
import { PolicyPanel } from '../../components/PolicyPanel';
import { installPolicyWriter } from '../../policyApi';
import { policyStatus } from '../../policyModel';
import { jsonResponse } from '../fixtures';
import { statusBody } from '../policy/policyFixtures';
import { agentsBody } from '../../../dev/fixtures/agents';
import { initialPolicy } from '../../../dev/fixtures/policy';
import { INSTANCE } from '../../../dev/fixtures/ids';
const now = Date.parse('2026-11-13T12:00:00Z');
let uninstall: (() => void) | undefined;
afterEach(() => {
  uninstall?.();
  uninstall = undefined;
});
function fixture(status = 200) {
  const policy = initialPolicy(now);
  const fetcher = vi.fn(async (input: RequestInfo | URL) => {
    const path = String(input);
    if (path === '/api/v1/agents')
      return jsonResponse(
        status === 200
          ? agentsBody(policy, now)
          : {
              code:
                status === 404 ? 'agents_plane_absent' : 'agents_unavailable',
            },
        status,
      );
    if (path.includes('/board/overview'))
      return jsonResponse({ code: 'absent' }, 404);
    if (path === '/api/v1/policy') return jsonResponse(statusBody());
    if (path === '/api/v1/policy/desired')
      return jsonResponse({ desired: [], newer_open: {}, blocked_by: {} });
    if (path === '/api/v1/policy/versions')
      return jsonResponse({ versions: [] });
    throw new Error(`unexpected ${path}`);
  });
  vi.stubGlobal('fetch', fetcher);
  return { policy, fetcher };
}
function mount(item: string | null = null) {
  window.history.replaceState(
    {},
    '',
    `#view=policy&panel=agents${item ? `&item=${item}` : ''}`,
  );
  return render(
    <StrictMode>
      <PolicyPanel
        initial={{
          status: policyStatus(statusBody()),
          desired: [],
          versions: [],
        }}
      />
    </StrictMode>,
  );
}
it('深链接保留旧实例并聚焦；撤销先预填和确认，成功后重读状态', async () => {
  const user = userEvent.setup(),
    { policy, fetcher } = fixture();
  const writer = vi.fn(async (path: string, body: unknown) => {
    const request = body as {
      level: 'instance';
      target: string;
      reason: string;
    };
    expect(path).toBe('/api/v1/agents/revoke');
    policy.revocations.push({
      seq: 99,
      ...request,
      revoked_at: new Date(now).toISOString(),
      revoked_by: 'owner',
      released_at: null,
      released_by: null,
    });
    return { created: true, level: request.level, target: request.target };
  });
  uninstall = installPolicyWriter(writer);
  mount(INSTANCE.REVOKED);
  const old = await screen.findByRole('article', {
    name: `实例 ${INSTANCE.REVOKED}`,
  });
  await waitFor(() => expect(old).toHaveFocus());
  expect(screen.getByRole('tab', { name: '接入方' })).toHaveAttribute(
    'aria-selected',
    'true',
  );
  const current = screen.getByRole('article', { name: `实例 ${INSTANCE.C1}` });
  await user.click(within(current).getByRole('button', { name: '撤销此实例' }));
  const form = screen.getByRole('form', { name: '撤销接入' });
  expect(within(form).getByLabelText('目标')).toHaveValue(INSTANCE.C1);
  await user.type(within(form).getByLabelText('理由'), '结束本次协作');
  await user.click(
    within(form).getByRole('button', { name: '撤销接入（即刻生效）' }),
  );
  expect(writer).not.toHaveBeenCalled();
  await user.click(within(form).getByRole('button', { name: /确认/ }));
  await waitFor(() => expect(writer).toHaveBeenCalledTimes(1));
  await waitFor(() =>
    expect(
      within(
        screen.getByRole('article', { name: `实例 ${INSTANCE.C1}` }),
      ).getByText('实例已撤销接入'),
    ).toBeInTheDocument(),
  );
  await waitFor(() =>
    expect(
      screen.getByRole('article', { name: `实例 ${INSTANCE.C1}` }),
    ).toHaveFocus(),
  );
  expect(
    fetcher.mock.calls.filter(([path]) => String(path) === '/api/v1/agents')
      .length,
  ).toBeGreaterThan(1);
});
it.each([
  [404, '本入口尚未接入身份列表。'],
  [503, '接入方身份或策略暂不可用，请稍后重试。'],
])('区分未装配与不可用 (%i)', async (status, message) => {
  fixture(status);
  mount();
  expect(await screen.findByText(message)).toBeInTheDocument();
  expect(screen.queryByText('尚未登记接入类型。')).not.toBeInTheDocument();
});
it('离开接入方标签时取消读取，迟到结果不重新挂载列表', async () => {
  let signal: AbortSignal | undefined;
  vi.stubGlobal(
    'fetch',
    vi.fn((_path, init: RequestInit) => {
      signal = init.signal as AbortSignal;
      return new Promise(() => {});
    }),
  );
  mount();
  await userEvent.click(screen.getByRole('tab', { name: '提交' }));
  expect(signal?.aborted).toBe(true);
  expect(screen.queryByText('正在读取接入方…')).not.toBeInTheDocument();
});
