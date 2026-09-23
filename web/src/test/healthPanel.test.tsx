import { render, screen, within } from '@testing-library/react';
import { expect, it } from 'vitest';
import { HealthPanel } from '../HealthPanel';
import { NO_RECOVERY_POINT, parseHealth } from '../healthApi';

function report(
  sections: Record<string, unknown>[],
  overall = 'normal',
  complete = true,
) {
  return parseHealth({ overall, evidence_complete: complete, sections });
}

it('健康页逐节展示标题状态与中文结论', () => {
  render(
    <HealthPanel
      report={report([
        {
          code: 'recovery-point',
          title: '恢复点年龄',
          state: 'normal',
          detail: '最新恢复点 100 秒前的内容，已在预算内',
        },
        {
          code: 'disk-quota',
          title: '磁盘配额',
          state: 'normal',
          detail: '占用在预警水位以内',
        },
      ])}
    />,
  );

  const list = screen.getByRole('list', { name: '健康检查项' });
  expect(within(list).getAllByRole('listitem')).toHaveLength(2);
  expect(list).toHaveTextContent('恢复点年龄');
  expect(list).toHaveTextContent('占用在预警水位以内');
});

it('未建立恢复点时该节显示未知并保留既定中文不显示正常', () => {
  render(
    <HealthPanel
      report={report(
        [
          {
            code: 'recovery-point',
            title: '恢复点年龄',
            state: 'unknown',
            detail: NO_RECOVERY_POINT,
          },
        ],
        'unknown',
        false,
      )}
    />,
  );

  expect(screen.getByText(NO_RECOVERY_POINT)).toBeInTheDocument();
  const states = screen.getAllByText('未知');
  expect(states.length).toBeGreaterThan(0);
  expect(screen.queryByText('正常')).not.toBeInTheDocument();
});

it('证据不完整时显式提示未知不等于一切正常', () => {
  render(
    <HealthPanel
      report={report(
        [
          {
            code: 'disk-quota',
            title: '磁盘配额',
            state: 'unknown',
            detail: '占用读数不可得，无法判断',
          },
        ],
        'unknown',
        false,
      )}
    />,
  );

  expect(
    screen.getByText(/证据不完整，缺失项一律显示未知/),
  ).toBeInTheDocument();
});

it('整体正常时不显示证据不完整提示', () => {
  render(
    <HealthPanel
      report={report([
        {
          code: 'disk-quota',
          title: '磁盘配额',
          state: 'normal',
          detail: '占用在预警水位以内',
        },
      ])}
    />,
  );

  expect(screen.queryByText(/证据不完整/)).not.toBeInTheDocument();
  expect(screen.getByRole('heading', { name: '磁盘配额' })).toBeInTheDocument();
});

it('降级状态使用降级语义而不是被渲染成正常', () => {
  render(
    <HealthPanel
      report={report(
        [
          {
            code: 'sync-quota',
            title: 'Sync 配额',
            state: 'degraded',
            detail: '占用已越过上限，对应写入路径已阻断',
          },
        ],
        'degraded',
      )}
    />,
  );

  const list = screen.getByRole('list', { name: '健康检查项' });
  expect(within(list).getByText('降级')).toHaveAttribute(
    'data-state',
    'degraded',
  );
});

it('检查项为空时页面仍然显示整体未知而不是空白的正常', () => {
  render(<HealthPanel report={report([], 'normal', true)} />);

  // 整体状态一行换成 StatusMark：健康原值在「整体状态」后面紧跟的元素上，状态字「未知」在它里面。
  const overall = screen.getByText('整体状态')
    .nextElementSibling as HTMLElement;
  expect(overall).toHaveAttribute('data-state', 'unknown');
  expect(within(overall).getByText('未知')).toBeInTheDocument();
  expect(
    screen.getByRole('list', { name: '健康检查项' }),
  ).toBeEmptyDOMElement();
});
