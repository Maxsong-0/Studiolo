import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import { ChangeList } from '../../components/PolicyReport';
import { report } from '../../policyModel';
import {
  policyServer,
  reportBody,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

const WIDENED = {
  path: 'read.project_whitelist',
  before: null,
  after: '项目/Semiment',
  note: '读取范围变宽',
};

describe('版本与回滚：发布时记下的扩大项是历史，不标红', () => {
  it('ChangeList 的 historical：仍以文字写出「扩大」，但用中性空心方块与中性色', () => {
    render(
      <ChangeList
        items={report(reportBody()).expansions}
        kind="expansion"
        historical
        label="v2 发布时的扩大项"
      />,
    );
    const list = screen.getByRole('list', { name: 'v2 发布时的扩大项' });
    expect(list).toHaveClass('policy-changes--history');
    expect(list).not.toHaveClass('policy-changes--expansion');
    const tag = within(list).getByText('扩大');
    expect(tag.closest('.ui-status')).toHaveAttribute('data-state', 'pending');
    expect(list.querySelector('.ui-signal-mark')).toBeNull();
  });

  it('版本行把发布时的扩大项写成中性文字；打开那一版看正文与记下的扩大项，版本号进位置', async () => {
    server = policyServer({
      versions: [versionBody(2, { expansions: [WIDENED] }), versionBody(1)],
    });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    const versions = await screen.findByRole('list', { name: '版本列表' });
    const open = within(versions).getByRole('button', { name: 'v2' });
    const row = open.closest('li');
    if (!row) throw new Error('缺少 v2 行');

    expect(row).toHaveTextContent('发布时扩大 1 项');
    expect(row.querySelector('.ui-status[data-state="alert"]')).toBeNull();
    // 回滚仍是红框的危险按钮：它是不可逆动作，不是扩大项。
    expect(
      within(row).getByRole('button', { name: '回滚到此版本（需二次认证）' }),
    ).toHaveClass('ui-button--danger');

    await user.click(open);
    expect(await screen.findByText('v2 正文')).toBeInTheDocument();
    expect(open).toHaveAttribute('aria-current', 'true');
    const recorded = screen.getByRole('list', { name: 'v2 发布时的扩大项' });
    expect(within(recorded).getByText('读取范围变宽')).toBeInTheDocument();
    expect(
      within(recorded).getByText('扩大').closest('.ui-status'),
    ).toHaveAttribute('data-state', 'pending');
    expect(window.location.hash).toContain('item=2');
    expect(window.location.hash).not.toContain('正文');
  });
});
