import { render, screen, waitFor } from '@testing-library/react';
import { afterEach, expect, it, vi } from 'vitest';
import { HealthBoundary } from '../HealthBoundary';

function ok() {
  return new Response(
    JSON.stringify({
      overall: 'normal',
      evidence_complete: true,
      sections: [
        {
          code: 'recovery-point',
          title: '恢复点年龄',
          state: 'normal',
          detail: '最新恢复点 100 秒前的内容，已在预算内',
        },
      ],
    }),
    { status: 200, headers: { 'Content-Type': 'application/json' } },
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

it('读取成功后展示健康页各节', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => ok()),
  );

  render(<HealthBoundary />);

  await waitFor(() =>
    expect(screen.getByRole('list', { name: '健康检查项' })).toHaveTextContent(
      '恢复点年龄',
    ),
  );
});

it('读取失败时显示未知并明确未知不等于正常', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(
      async () =>
        new Response(JSON.stringify({ error: '本机视图暂不可用，请重试。' }), {
          status: 503,
          headers: { 'Content-Type': 'application/json' },
        }),
    ),
  );

  render(<HealthBoundary />);

  await waitFor(() =>
    expect(screen.getByText(/未知不等于正常/)).toBeInTheDocument(),
  );
  expect(screen.queryByText('正常')).not.toBeInTheDocument();
});

it('会话失效时也只显示未知不会退化成正常', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => new Response('', { status: 401 })),
  );

  render(<HealthBoundary />);

  await waitFor(() =>
    expect(screen.getByText(/登录已失效/)).toBeInTheDocument(),
  );
});

it('卸载后到达的响应不再写入状态', async () => {
  const pending: ((value: Response) => void)[] = [];
  vi.stubGlobal(
    'fetch',
    vi.fn(
      () =>
        new Promise<Response>((done) => {
          pending.push(done);
        }),
    ),
  );
  const view = render(<HealthBoundary />);
  expect(screen.getByText(/正在读取本机健康证据/)).toBeInTheDocument();

  view.unmount();
  for (const done of pending) done(ok());

  await waitFor(() =>
    expect(screen.queryByRole('list', { name: '健康检查项' })).toBeNull(),
  );
});
