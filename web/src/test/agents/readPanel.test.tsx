import { StrictMode } from 'react';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, expect, it, vi } from 'vitest';
import { ReadAuditPanel } from '../../agents/ReadAuditPanel';
import { installPolicyWriter } from '../../policyApi';
import { ApiError } from '../../sessionHttp';
import { readAuditBody } from '../../../dev/fixtures/readAudit';
let uninstall: (() => void) | undefined;
afterEach(() => {
  uninstall?.();
  uninstall = undefined;
});
function mount() {
  return render(
    <StrictMode>
      <ReadAuditPanel types={['claude', 'codex']} />
    </StrictMode>,
  );
}
it('仅提交才查询，分页沿同一筛选，拒绝记录禁用路径条件', async () => {
  const user = userEvent.setup(),
    now = Date.now();
  const writer = vi.fn(async (_path: string, body: unknown) =>
    readAuditBody(body as Record<string, unknown>, now),
  );
  uninstall = installPolicyWriter(writer);
  mount();
  expect(writer).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: '查询审计' }));
  await screen.findByText(/本页检查 256 条账本记录/);
  expect(writer.mock.calls[0][0]).toBe('/api/v1/agents/reads/query');
  await user.click(screen.getByRole('button', { name: '更早记录' }));
  await waitFor(() => expect(writer).toHaveBeenCalledTimes(2));
  expect(writer.mock.calls[1][1]).toMatchObject({ before: 265 });
  await user.click(screen.getByRole('button', { name: '上一页' }));
  await waitFor(() => expect(writer).toHaveBeenCalledTimes(3));
  expect(writer.mock.calls[2][1]).toMatchObject({ before: 521 });
  await user.selectOptions(screen.getByLabelText('记录类别'), 'denied');
  expect(screen.getByLabelText('路径前缀')).toBeDisabled();
  await user.click(screen.getByRole('button', { name: '查询审计' }));
  await screen.findAllByText('capability_denied');
  expect(writer.mock.calls[3][1]).toMatchObject({
    outcome: 'denied',
    before: null,
    path_prefix: null,
  });
});
it('稀疏空页与到达起点不同，条件变化和取消会作废迟到请求', async () => {
  const user = userEvent.setup(),
    now = Date.now();
  const writer = vi.fn(async () =>
    readAuditBody({ outcome: 'read', path_prefix: 'missing' }, now),
  );
  uninstall = installPolicyWriter(writer);
  const view = mount();
  await user.click(screen.getByRole('button', { name: '查询审计' }));
  await screen.findByText(/继续查询更早记录，不能据此判断全库没有匹配/);
  expect(screen.getByRole('button', { name: '更早记录' })).toBeEnabled();
  let finish!: (v: unknown) => void;
  let signal!: AbortSignal;
  uninstall();
  uninstall = installPolicyWriter((_p, _b, s) => {
    signal = s;
    return new Promise((resolve) => {
      finish = resolve;
    });
  });
  await user.click(screen.getByRole('button', { name: '查询审计' }));
  await user.click(screen.getByRole('button', { name: '取消查询' }));
  expect(signal.aborted).toBe(true);
  finish(readAuditBody({ outcome: 'read' }, now));
  expect(screen.queryByText(/本页检查/)).not.toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '查询审计' }));
  view.unmount();
  expect(signal.aborted).toBe(true);
  finish(readAuditBody({ outcome: 'read' }, now));
});
it.each([404, 503])('HTTP %s 不伪装为空审计', async (status) => {
  uninstall = installPolicyWriter(async () => {
    throw new ApiError('失败', status);
  });
  mount();
  await userEvent
    .setup()
    .click(screen.getByRole('button', { name: '查询审计' }));
  expect(await screen.findByRole('alert')).toHaveTextContent(
    status === 404 ? '尚未接入' : '暂不可用',
  );
  expect(screen.queryByText('本页没有匹配记录。')).not.toBeInTheDocument();
});
