import { render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { PolicyReport } from '../../components/PolicyReport';
import { report } from '../../policyModel';
import { reportBody } from './policyFixtures';

function counts() {
  const element = document.querySelector('.policy-report__counts');
  if (!(element instanceof HTMLElement)) throw new Error('缺少报告计数');
  return element;
}

describe('校验报告', () => {
  it('扩大项行首是红色实心方块，并以文字「扩大」标出；收窄项是空心方块与中性色「收窄」', () => {
    render(<PolicyReport report={report(reportBody())} />);

    const expansions = screen.getByRole('list', { name: '权限扩大项' });
    const expansion = within(expansions).getByRole('listitem');
    const tag = within(expansion).getByText('扩大');
    // 「扩大」是文字，不只靠颜色；方块是 alert 态（红色实心），且排在行首。
    expect(tag.closest('.ui-status')).toHaveAttribute('data-state', 'alert');
    expect(expansion.firstElementChild).toBe(tag.closest('.ui-status'));
    expect(expansions).toHaveClass('policy-changes--expansion');
    expect(within(expansion).getByText('读取范围变宽')).toBeInTheDocument();
    expect(within(expansion).getByText(/改为：项目\/Semiment/)).toBeVisible();

    const narrowings = screen.getByRole('list', { name: '权限收窄项' });
    const narrowing = within(narrowings).getByRole('listitem');
    expect(
      within(narrowing).getByText('收窄').closest('.ui-status'),
    ).toHaveAttribute('data-state', 'pending');
    expect(narrowings).toHaveClass('policy-changes--narrowing');
    expect(narrowings.querySelector('.ui-signal-mark')).toBeNull();

    // 有扩大项才发 alert，提醒发布前逐条确认。
    expect(screen.getByRole('alert')).toHaveTextContent(
      '1 项权限扩大，发布前逐条确认：',
    );
  });

  it('报告标题右侧用等宽计数「扩大 N · 收窄 N · 未知 —」；接口不单列未知项，写明原因而不补数', () => {
    render(<PolicyReport report={report(reportBody())} />);
    expect(
      screen.getByRole('heading', { level: 4, name: '校验报告' }),
    ).toBeInTheDocument();
    const line = counts();
    expect(line).toHaveTextContent('扩大 1 · 收窄 1 · 未知 —');
    // 扩大数大于零：红字并带红色方块。
    const expansion = within(line).getByText('扩大 1');
    expect(expansion).toHaveClass('is-signal');
    expect(expansion.querySelector('.ui-signal-mark')).not.toBeNull();
    // 未知拿不到：显示「—」，原因进 title，并给读屏一份。
    const unknown = within(line).getByTitle(/接口没有单列未知项/);
    expect(unknown).toHaveTextContent('未知 —');
    expect(unknown.querySelector('.sr-only')).toHaveTextContent(
      /接口没有单列未知项/,
    );
  });

  it('没有扩大项时不标红、不发 alert', () => {
    render(
      <PolicyReport
        report={report(reportBody({ expansions: [], narrowings: [] }))}
      />,
    );
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    expect(screen.getByText('没有权限扩大项。')).toBeInTheDocument();
    expect(within(counts()).getByText('扩大 0')).not.toHaveClass('is-signal');
    expect(counts().querySelector('.ui-signal-mark')).toBeNull();
  });

  it('校验问题以文字「问题」加红色方块标出，结论为校验未通过', () => {
    render(
      <PolicyReport
        report={report(
          reportBody({
            ok: false,
            problems: [{ path: 'policy.activate', message: '永久为 false' }],
            expansions: [],
          }),
        )}
      />,
    );
    const problems = screen.getByRole('list', { name: '校验问题' });
    const item = within(problems).getByRole('listitem');
    expect(
      within(item).getByText('问题').closest('.ui-status'),
    ).toHaveAttribute('data-state', 'alert');
    expect(item).toHaveTextContent('policy.activate 永久为 false');
    expect(
      screen.getByText('校验未通过').closest('.ui-status'),
    ).toHaveAttribute('data-state', 'alert');
  });

  it('差异只列改动：符号区分增删改（读屏读出文字），不带任何红色标记', () => {
    render(<PolicyReport report={report(reportBody())} />);
    const diff = document.querySelector('details.policy-diff');
    if (!(diff instanceof HTMLElement)) throw new Error('缺少差异');
    expect(diff.querySelector('summary')).toHaveTextContent(
      '差异 diff（json）：＋1 －1 ～0',
    );
    const entries = within(diff).getAllByRole('listitem', { hidden: true });
    expect(entries.map((entry) => entry.dataset.op)).toEqual([
      'added',
      'removed',
    ]);
    expect(entries[0]).toHaveTextContent('＋新增');
    expect(entries[1]).toHaveTextContent('－删除');
    expect(diff.querySelector('.ui-signal-mark, .ui-status')).toBeNull();
  });
});
