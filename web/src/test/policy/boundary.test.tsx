import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import { jsonResponse } from '../fixtures';
import { statusBody } from './policyFixtures';

const fetchMock = vi.fn<typeof fetch>();
const reads = () => fetchMock.mock.calls.map(([input]) => String(input));

function snapshot(path: string) {
  if (path === '/api/v1/policy') return jsonResponse(statusBody());
  if (path === '/api/v1/policy/desired') return jsonResponse({ desired: [] });
  if (path === '/api/v1/policy/versions') return jsonResponse({ versions: [] });
  return jsonResponse({ error: '未知' }, 404);
}

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => {
  vi.unstubAllGlobals();
});

describe('策略平面边界的三态', () => {
  it('读取中：占位条与「正在读取策略平面…」（role=status），不是空白', async () => {
    let release: () => void = () => undefined;
    const gate = new Promise<void>((resolve) => {
      release = resolve;
    });
    fetchMock.mockImplementation(async (input) => {
      await gate;
      return snapshot(String(input));
    });
    render(<PolicyBoundary />);
    const region = screen.getByRole('region', { name: '策略平面' });
    expect(within(region).getByRole('status')).toHaveTextContent(
      '正在读取策略平面…',
    );
    release();
    expect(
      await screen.findByRole('table', { name: '生效中的各项' }),
    ).toBeInTheDocument();
    expect(screen.queryByText('正在读取策略平面…')).not.toBeInTheDocument();
  });

  it('未接入：空态「未接入策略平面」加原因原文，不当成错误', async () => {
    fetchMock.mockImplementation(async () =>
      jsonResponse({ code: 'policy_plane_absent', error: '未装配' }, 404),
    );
    render(<PolicyBoundary />);
    expect(await screen.findByText('未接入策略平面')).toBeInTheDocument();
    expect(
      screen.getByText(/本入口未装配策略平面（启动时未给 --policy-data）/),
    ).toBeInTheDocument();
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(reads()).toEqual(['/api/v1/policy']);
  });

  it('读取失败：错误态给出原文；只有人点「重新读取」才再读一次整份快照', async () => {
    let failing = true;
    fetchMock.mockImplementation(async (input) =>
      failing
        ? jsonResponse({ error: '读取不可用' }, 503)
        : snapshot(String(input)),
    );
    const user = userEvent.setup();
    render(<PolicyBoundary />);

    const alert = await screen.findByRole('alert');
    expect(alert).toHaveTextContent('读取策略平面失败');
    expect(alert).toHaveTextContent('读取不可用');
    // 不自动重试：首项失败就停，其余列表不读。
    expect(reads()).toEqual(['/api/v1/policy']);

    failing = false;
    await user.click(within(alert).getByRole('button', { name: '重新读取' }));
    expect(
      await screen.findByRole('table', { name: '生效中的各项' }),
    ).toBeInTheDocument();
    expect(reads()).toEqual([
      '/api/v1/policy',
      '/api/v1/policy',
      '/api/v1/policy/desired',
      '/api/v1/policy/versions',
    ]);
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
  });
});
