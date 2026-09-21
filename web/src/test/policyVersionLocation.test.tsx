import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { PolicyBoundary } from '../PolicyBoundary';
import { jsonResponse } from './fixtures';

const HASH = 'c'.repeat(64);
const STATUS = '/api/v1/policy';

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

const fetchMock = vi.fn<typeof fetch>();
const reads: string[] = [];

beforeEach(() => {
  reads.length = 0;
  fetchMock.mockReset();
  fetchMock.mockImplementation(async (input) => {
    const path = String(input);
    reads.push(path);
    if (path === STATUS)
      return jsonResponse({
        effective_hash: HASH,
        entries: [],
        global_revoked: false,
        pending_desired: 0,
        revocations: [],
        l3_available: true,
      });
    if (path === '/api/v1/policy/desired') return jsonResponse({ desired: [] });
    if (path === '/api/v1/policy/versions')
      return jsonResponse({ versions: [meta(2), meta(1)] });
    if (path === '/api/v1/policy/versions/2')
      return jsonResponse({ version: { meta: meta(2), body: '第二版正文' } });
    if (path === '/api/v1/policy/versions/1')
      return jsonResponse({ version: { meta: meta(1), body: '第一版正文' } });
    return jsonResponse({ error: '未知' }, 404);
  });
  vi.stubGlobal('fetch', fetchMock);
});
afterEach(() => {
  vi.unstubAllGlobals();
});

it('打开某一版的正文会把版本号记进位置，正文本身不进 URL', async () => {
  const user = userEvent.setup();
  render(<PolicyBoundary />);
  await screen.findByRole('list', { name: '版本列表' });

  await user.click(screen.getByRole('button', { name: 'v2' }));
  expect(await screen.findByText('第二版正文')).toBeInTheDocument();
  expect(window.location.hash).toContain('item=2');
  expect(window.location.hash).not.toContain('第二版正文');
  // 正文只读一次：位置变化不应让回读 effect 再发一遍。
  expect(reads.filter((path) => path === '/api/v1/policy/versions/2')).toEqual([
    '/api/v1/policy/versions/2',
  ]);
});

it('位置里带着版本号就直接回到那一份 diff（重挂、刷新或分享链接）', async () => {
  window.location.hash = '#view=policy&item=1';
  render(<PolicyBoundary />);

  expect(await screen.findByText('第一版正文')).toBeInTheDocument();
  // 回读不抢通知位：这是恢复位置，不是人刚点的动作。
  expect(screen.queryByText('已读取 v1。')).not.toBeInTheDocument();
});

it('位置里的版本号不在列表里就停在列表，不猜别的版本也不清位置', async () => {
  window.location.hash = '#view=policy&item=99';
  render(<PolicyBoundary />);
  await screen.findByRole('list', { name: '版本列表' });

  await waitFor(() =>
    expect(
      reads.some((path) => path.startsWith('/api/v1/policy/versions/')),
    ).toBe(false),
  );
  expect(screen.queryByText(/版正文/)).not.toBeInTheDocument();
  expect(window.location.hash).toContain('item=99');
});
