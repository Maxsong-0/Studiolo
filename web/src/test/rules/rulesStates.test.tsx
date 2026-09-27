import { act, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { RulesView } from '../../views/RulesView';
import { jsonResponse } from '../fixtures';

async function open(hash = '#view=rules') {
  window.location.hash = hash;
  await act(async () => {
    render(<RulesView />);
  });
}

it('未接入策略平面（404 policy_plane_absent）：写「未接入」，不当成零条规则', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () =>
      jsonResponse(
        { code: 'policy_plane_absent', error: '本入口未装配策略平面。' },
        404,
      ),
    ),
  );
  await open();
  expect(await screen.findByText('未接入策略平面')).toBeInTheDocument();
  expect(screen.queryByRole('table')).not.toBeInTheDocument();
});

it('版本记录与生效条目对不上：整份作废并写明原因，不显示半份数据；点「重新读取」才再读', async () => {
  const api = createFixtureApi();
  let tamper = true;
  const fetchMock = vi.fn(async (input: RequestInfo | URL) => {
    const url = String(input);
    const outcome = api.handle({ method: 'GET', url, headers: {} });
    if (tamper && url === '/api/v1/policy/versions/3') {
      const { version } = outcome.body as {
        version: { meta: Record<string, unknown>; body: string | null };
      };
      return jsonResponse({
        version: {
          ...version,
          meta: { ...version.meta, body_sha256: 'f'.repeat(64) },
        },
      });
    }
    return jsonResponse(outcome.body ?? null, outcome.status);
  });
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  await open('#view=rules&panel=access');
  expect(await screen.findByRole('alert')).toHaveTextContent(
    '规则正文与生效条目对不上',
  );
  expect(screen.queryByRole('switch')).not.toBeInTheDocument();
  const reads = fetchMock.mock.calls.length;
  tamper = false;
  await user.click(screen.getByRole('button', { name: '重新读取' }));
  expect(
    await screen.findByRole('switch', { name: 'claude 读取 CLAUDE.md' }),
  ).toHaveAttribute('aria-checked', 'true');
  expect(fetchMock.mock.calls.length).toBeGreaterThan(reads);
});
