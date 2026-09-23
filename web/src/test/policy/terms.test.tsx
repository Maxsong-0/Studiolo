import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it } from 'vitest';
import { PolicyBoundary } from '../../PolicyBoundary';
import {
  CHANGE_TERMS,
  POLICY_TERMS,
  REVOKE_LEVEL_TERMS,
} from '../../policy/terms';
import {
  desiredBody,
  policyServer,
  statusBody,
  versionBody,
  type PolicyServer,
} from './policyFixtures';

let server: PolicyServer | undefined;
afterEach(() => {
  server?.uninstall();
  server = undefined;
});

/** 所有原术语，一个都不许出现在按钮的可访问名里。 */
const RAW_TERMS = [
  ...Object.values(POLICY_TERMS),
  ...Object.values(REVOKE_LEVEL_TERMS),
  ...Object.values(CHANGE_TERMS),
].map((term) => term.raw);
const escape = (value: string) => value.replace(/[.*+?^${}()|[\]\\-]/g, '\\$&');
const RAW_IN_NAME = new RegExp(RAW_TERMS.map(escape).join('|'), 'i');

describe('策略术语表', () => {
  it('收录任务书要求的每一组「原术语 → 人话」', () => {
    const table = Object.fromEntries(
      Object.values(POLICY_TERMS).map((term) => [term.raw, term.human]),
    );
    expect(table).toEqual({
      desired: '规则草案',
      effective: '生效中',
      'One-time Code': '一次性验证码',
      revoke: '撤销接入',
      'step-up': '二次认证',
      subject: '对象',
      sha256: '摘要',
      diff: '差异',
      validate: '校验',
      activate: '发布',
      rollback: '回滚',
    });
    expect(Object.values(REVOKE_LEVEL_TERMS).map((term) => term.human)).toEqual(
      ['实例', '类型', '全局'],
    );
    expect(Object.values(CHANGE_TERMS).map((term) => term.human)).toEqual([
      '扩大',
      '收窄',
      '未知',
    ]);
  });

  it('格子标题与表头先写人话，原术语用等宽小字（code.policy-term）跟在后面', async () => {
    server = policyServer({
      desired: [desiredBody()],
      versions: [versionBody(2)],
    });
    render(<PolicyBoundary />);
    const table = await screen.findByRole('table', { name: '生效中的各项' });

    for (const [human, raw] of [
      ['生效中', 'effective'],
      ['提交规则草案', 'desired'],
      ['待处理草案', 'desired'],
      ['版本与回滚', 'rollback'],
      ['撤销接入（实例/类型/全局）', 'revoke'],
    ] as const) {
      const heading = screen.getByRole('heading', {
        level: 2,
        name: `${human} ${raw}`,
      });
      expect(heading.textContent?.startsWith(human)).toBe(true);
      const term = heading.querySelector('code.policy-term');
      expect(term).toHaveTextContent(raw);
      // 原术语跟在人话后面：它是标题里最后一段文字。
      expect(heading.textContent?.endsWith(raw)).toBe(true);
    }

    for (const [human, raw] of [
      ['对象', 'subject'],
      ['摘要', 'sha256'],
    ] as const) {
      const header = within(table).getByRole('columnheader', {
        name: `${human} ${raw}`,
      });
      expect(header.querySelector('code.policy-term')).toHaveTextContent(raw);
    }

    // 字段：输入框的可访问名只是人话，原术语等宽跟在字段名后面。
    const form = screen.getByRole('form', { name: '提交规则草案' });
    const subject = within(form).getByRole('textbox', { name: '对象' });
    const label = subject
      .closest('.policy-field')
      ?.querySelector('.policy-field__label');
    expect(label).toHaveTextContent('对象 subject');
    expect(label?.querySelector('code.policy-term')).toHaveTextContent(
      'subject',
    );
  });

  it('按钮的可访问名只用人话，不带任何原术语', async () => {
    server = policyServer({
      status: statusBody({ global_revoked: true }),
      desired: [desiredBody()],
      versions: [versionBody(2), versionBody(1)],
    });
    const user = userEvent.setup();
    render(<PolicyBoundary />);
    await screen.findByRole('table', { name: '生效中的各项' });
    // 把能出现的按钮都摆出来：选中草案（发布、重新校验、撤回），展开一个执行预览（确认、取消）。
    await user.click(screen.getByRole('button', { name: '能力集 claude' }));
    await user.click(
      screen.getByRole('button', { name: '发布（需二次认证）' }),
    );
    expect(
      screen.getByRole('button', { name: '确认发布' }),
    ).toBeInTheDocument();

    const buttons = screen.getAllByRole('button');
    expect(buttons.length).toBeGreaterThan(8);
    expect(screen.queryAllByRole('button', { name: RAW_IN_NAME })).toEqual([]);
  });
});
