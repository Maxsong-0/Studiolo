import { act, screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import {
  codes,
  factorResponse,
  factorSnapshot,
  mockFactors,
  readyFactors,
  recent,
} from './factorFixtures';
import { openFactorAccount } from './factorUi';

describe('账户内安全操作', () => {
  it('二次认证提交即清空输入，并与会话退出互斥；成功后重新核对期限', async () => {
    const fetchMock = mockFactors();
    const pending = deferred<Response>();
    let verified = false;
    fetchMock.mockImplementation(async (input, init) => {
      if (input === '/api/v1/auth/state')
        return jsonResponse({
          factors: verified ? readyFactors : factorSnapshot,
        });
      if (input === '/api/v1/auth/step-up') return pending.promise;
      return factorResponse(input, init);
    });
    const { user } = await openFactorAccount();
    expect(screen.getByRole('button', { name: '生成恢复码' })).toBeDisabled();
    await user.click(screen.getByRole('button', { name: '二次认证' }));
    const input = screen.getByLabelText('验证码');
    await user.type(input, '123456');
    await user.click(screen.getByRole('button', { name: '确认二次认证' }));
    expect(input).toHaveValue('');
    expect(screen.getByRole('button', { name: '退出登录' })).toBeDisabled();
    expect(screen.getByRole('button', { name: '确认二次认证' })).toBeDisabled();
    await act(async () => {
      verified = true;
      pending.resolve(jsonResponse(recent));
    });
    await waitFor(() =>
      expect(screen.getByRole('button', { name: '生成恢复码' })).toBeEnabled(),
    );
    expect(screen.getByText(/已完成二次认证，有效至/)).toBeInTheDocument();
    expect(
      fetchMock.mock.calls.filter(([, init]) => init?.method === 'POST'),
    ).toHaveLength(1);
  });

  it('换发需要明确确认，恢复码只交付本次且仅点击时复制', async () => {
    const fetchMock = mockFactors();
    const { user } = await openFactorAccount();
    const copy = vi
      .spyOn(navigator.clipboard, 'writeText')
      .mockResolvedValue(undefined);
    await user.click(screen.getByRole('button', { name: '生成恢复码' }));
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === 'POST'),
    ).toBe(false);
    expect(screen.getByText(/之前所有恢复码立即失效/)).toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '确认生成新恢复码' }));
    const delivery = await screen.findByRole('list', { name: '新恢复码' });
    expect(within(delivery).getAllByRole('listitem')).toHaveLength(10);
    expect(delivery).toHaveTextContent(codes[0]);
    expect(copy).not.toHaveBeenCalled();
    await user.click(screen.getByRole('button', { name: '复制恢复码' }));
    expect(copy).toHaveBeenCalledOnce();
    expect(copy).toHaveBeenCalledWith(codes.join('\n'));
    await user.click(screen.getByRole('button', { name: '关闭账户与会话' }));
    expect(screen.queryByText(codes[0])).not.toBeInTheDocument();
    await user.click(screen.getByRole('button', { name: '账户与会话' }));
    await screen.findByRole('button', { name: '生成恢复码' });
    expect(
      screen.queryByRole('list', { name: '新恢复码' }),
    ).not.toBeInTheDocument();
    expect(
      fetchMock.mock.calls.filter(
        ([path]) => path === '/api/v1/auth/recovery-codes',
      ),
    ).toHaveLength(1);
  });

  it('恢复提交清空两个输入，204 后关闭内容和弹层，登录需再次明确操作', async () => {
    const fetchMock = mockFactors();
    const pending = deferred<Response>();
    fetchMock.mockImplementation(async (input, init) =>
      input === '/api/v1/auth/factor/recover'
        ? pending.promise
        : factorResponse(input, init),
    );
    const { user } = await openFactorAccount();
    await user.click(screen.getByRole('button', { name: '恢复验证器' }));
    const recovery = screen.getByLabelText('恢复码');
    const otp = screen.getByLabelText('新验证器验证码');
    await user.type(recovery, codes[0]);
    await user.type(otp, '123456');
    await user.click(screen.getByRole('button', { name: '确认恢复验证器' }));
    expect(recovery).toHaveValue('');
    expect(otp).toHaveValue('');
    await act(async () => pending.resolve(new Response(null, { status: 204 })));
    expect(
      await screen.findByRole('button', { name: '登录工作台' }),
    ).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect(screen.queryByText('已授权内容')).not.toBeInTheDocument();
    const posts = fetchMock.mock.calls.filter(
      ([, init]) => init?.method === 'POST',
    );
    expect(posts).toHaveLength(1);
    expect(JSON.parse(String(posts[0][1]?.body))).toEqual({
      recovery_code: codes[0],
      code: '123456',
    });
  });

  it('输入不规范时清空内容并说明格式，不修剪或截断后发请求', async () => {
    const fetchMock = mockFactors();
    const { user } = await openFactorAccount();
    await user.click(screen.getByRole('button', { name: '二次认证' }));
    const input = screen.getByLabelText('验证码');
    await user.type(input, '1234567');
    await user.click(screen.getByRole('button', { name: '确认二次认证' }));
    expect(await screen.findByRole('alert')).toHaveTextContent(
      '请输入六位数字验证码',
    );
    expect(input).toHaveValue('');
    expect(
      fetchMock.mock.calls.some(([, init]) => init?.method === 'POST'),
    ).toBe(false);
  });
});
