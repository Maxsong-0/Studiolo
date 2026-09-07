import { act, screen, waitFor } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { SessionClient } from '../sessionClient';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import { codes, factorResponse, mockFactors } from './factorFixtures';
import { openFactorAccount } from './factorUi';

describe('安全弹层清理', () => {
  it('关闭账户清除尚未提交的恢复码和验证码，包括已脱离 DOM 的输入', async () => {
    mockFactors();
    const { user } = await openFactorAccount();
    await user.click(screen.getByRole('button', { name: '恢复验证器' }));
    const recovery = screen.getByLabelText('恢复码');
    const otp = screen.getByLabelText('新验证器验证码');
    await user.type(recovery, codes[0]);
    await user.type(otp, '123456');
    await user.click(screen.getByRole('button', { name: '关闭账户与会话' }));
    expect(recovery).toHaveValue('');
    expect(otp).toHaveValue('');
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
  });

  it.each(['关闭账户与会话', '关闭此操作'])(
    '%s 在途换发立即清理，不交付迟到原码',
    async (close) => {
      const fetchMock = mockFactors();
      const pending = deferred<Response>();
      fetchMock.mockImplementation(async (input, init) =>
        input === '/api/v1/auth/recovery-codes'
          ? pending.promise
          : factorResponse(input, init),
      );
      const { user } = await openFactorAccount();
      await user.click(screen.getByRole('button', { name: '生成恢复码' }));
      await user.click(
        screen.getByRole('button', { name: '确认生成新恢复码' }),
      );
      await user.click(screen.getByRole('button', { name: close }));
      expect(await screen.findByRole('alert')).toHaveTextContent(
        '恢复码可能已经换发',
      );
      expect(screen.queryByText('已授权内容')).not.toBeInTheDocument();
      await act(async () => pending.resolve(jsonResponse({ codes })));
      expect(
        screen.queryByRole('list', { name: '新恢复码' }),
      ).not.toBeInTheDocument();
      expect(screen.queryByText(codes[0])).not.toBeInTheDocument();
      expect(
        fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
      ).toHaveLength(1);
    },
  );

  it.each(['suspend', 'changed'] as const)(
    '%s 后交付原码立即消失，恢复会话也不能重读',
    async (event) => {
      const fetchMock = mockFactors();
      const client = new SessionClient();
      const { user } = await openFactorAccount(client);
      await user.click(screen.getByRole('button', { name: '生成恢复码' }));
      await user.click(
        screen.getByRole('button', { name: '确认生成新恢复码' }),
      );
      await screen.findByRole('list', { name: '新恢复码' });
      act(() => client[event]());
      expect(screen.queryByText(codes[0])).not.toBeInTheDocument();
      if (event === 'suspend') act(() => client.resume());
      await user.click(
        await screen.findByRole('button', { name: '账户与会话' }),
      );
      await waitFor(() =>
        expect(screen.queryByText('正在核对安全操作…')).not.toBeInTheDocument(),
      );
      expect(
        screen.queryByRole('list', { name: '新恢复码' }),
      ).not.toBeInTheDocument();
      expect(
        fetchMock.mock.calls.filter(
          ([path]) => path === '/api/v1/auth/recovery-codes',
        ),
      ).toHaveLength(1);
    },
  );
});
