import { screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { jsonResponse } from './fixtures';
import { factorResponse, factorSnapshot, mockFactors } from './factorFixtures';
import { openFactorAccount } from './factorUi';

describe('安全操作错误与显式核对', () => {
  it.each([
    { status: 200, state: { factors: null }, message: '当前入口未启用' },
    { status: 404, state: {}, message: '暂时无法确认' },
    { status: 200, state: { factors: {} }, message: '暂时无法确认' },
  ])(
    '能力响应 $status / $message 时关闭动作，保留已授权内容',
    async ({ status, state, message }) => {
      const fetchMock = mockFactors();
      fetchMock.mockImplementation(async (input, init) =>
        input === '/api/v1/auth/state'
          ? jsonResponse(state, status)
          : factorResponse(input, init),
      );
      await openFactorAccount();
      expect(screen.getByText(new RegExp(message))).toBeInTheDocument();
      expect(screen.getByText('已授权内容')).toBeInTheDocument();
      expect(
        screen.queryByRole('button', { name: '二次认证' }),
      ).not.toBeInTheDocument();
      expect(
        fetchMock.mock.calls.some(([, init]) => init?.method === 'POST'),
      ).toBe(false);
    },
  );

  it.each([403, 409, 429])(
    '%s 后没有自动重放，状态 pending 仍允许显式新认证',
    async (status) => {
      const fetchMock = mockFactors();
      fetchMock.mockImplementation(async (input, init) => {
        if (input === '/api/v1/auth/state')
          return jsonResponse({
            factors: { ...factorSnapshot, pending: true },
          });
        if (input === '/api/v1/auth/step-up') return jsonResponse({}, status);
        return factorResponse(input, init);
      });
      const { user } = await openFactorAccount();
      await user.click(screen.getByRole('button', { name: '二次认证' }));
      const input = screen.getByLabelText('验证码');
      await user.type(input, '123456');
      await user.click(screen.getByRole('button', { name: '确认二次认证' }));
      expect(await screen.findByRole('alert')).toHaveTextContent(/核对/);
      expect(input).toHaveValue('');
      expect(screen.getByText('已授权内容')).toBeInTheDocument();
      expect(screen.queryByLabelText('验证码')).not.toBeInTheDocument();
      expect(
        fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
      ).toHaveLength(1);
      await user.click(screen.getByRole('button', { name: '核对安全状态' }));
      await user.click(await screen.findByRole('button', { name: '二次认证' }));
      expect(
        screen.getByText(/状态不会确认先前操作的结果/),
      ).toBeInTheDocument();
      await user.type(screen.getByLabelText('验证码'), '654321');
      await user.click(screen.getByRole('button', { name: '确认二次认证' }));
      await waitFor(() =>
        expect(
          fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
        ).toHaveLength(2),
      );
    },
  );

  it('503 换发结果先关闭内容，核对原状态也不声称旧动作失败或重读原码', async () => {
    const fetchMock = mockFactors();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/recovery-codes'
        ? jsonResponse({}, 503)
        : factorResponse(input, init),
    );
    const { user } = await openFactorAccount();
    await user.click(screen.getByRole('button', { name: '生成恢复码' }));
    await user.click(screen.getByRole('button', { name: '确认生成新恢复码' }));
    expect(await screen.findByRole('alert')).toHaveTextContent('原码无法重读');
    expect(screen.queryByText('已授权内容')).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '核对登录状态' }));
    await user.click(await screen.findByRole('button', { name: '账户与会话' }));
    await screen.findByRole('button', { name: '生成恢复码' });
    expect(screen.getByText(/再次生成是独立的新操作/)).toBeInTheDocument();
    expect(
      screen.queryByRole('list', { name: '新恢复码' }),
    ).not.toBeInTheDocument();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
    await user.click(screen.getByRole('button', { name: '生成恢复码' }));
    expect(
      screen.getByRole('button', { name: '确认生成新恢复码' }),
    ).toBeEnabled();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
  });
});
