import { render, screen, within } from '@testing-library/react';
import { expect, it } from 'vitest';
import { TitleBlock, type TitleBlockCell } from '../../ui';

const twelve: TitleBlockCell[] = [
  { label: '副本新鲜度', value: '111 s' },
  {
    label: '每轮同步',
    value: '85 s',
    tone: 'signal',
    reason: '超过时限的七成',
  },
  { label: '同步时限', value: '120 s' },
  { label: '黑板在跑', value: null, reason: '黑板未启用（404）' },
  { label: 'agent 程序', value: 't108-864cfb9' },
  { label: '人侧发行', value: 'r5' },
  { label: '会话', value: '今日已复核' },
  { label: '执行', value: null },
  { label: '可读项目', value: 22 },
  { label: '副本文件', value: '1,382' },
  { label: '副本大小', value: '219 MB' },
  { label: '未关闭 BUG', value: 0 },
];

const cellOf = (label: string) =>
  screen.getByText(label).closest('li') as HTMLElement;

it('拿不到的值显示「—」，原因进格子的 title 并给读屏一份', () => {
  render(
    <TitleBlock
      title="驾驶舱"
      subtitle={'2026-09-22 周二\n副本代次 243'}
      cells={twelve}
    />,
  );
  const board = cellOf('黑板在跑');
  expect(within(board).getByText('—')).toBeInTheDocument();
  expect(board).toHaveAttribute('title', '黑板未启用（404）');
  expect(within(board).getByText('黑板未启用（404）')).toHaveClass('sr-only');

  const execution = cellOf('执行');
  expect(within(execution).getByText('—')).toBeInTheDocument();
  expect(execution).not.toHaveAttribute('title');

  // 0 是真读数，不能被当成缺失
  expect(within(cellOf('未关闭 BUG')).getByText('0')).toBeInTheDocument();
  expect(within(cellOf('可读项目')).getByText('22')).toBeInTheDocument();
  expect(screen.getAllByText('—')).toHaveLength(2);
});

it('图名、日期与十二格读数都在；十二格正好排满三行不补空格', () => {
  const { container } = render(
    <TitleBlock
      title="驾驶舱"
      subtitle={'2026-09-22 周二\n副本代次 243'}
      cells={twelve}
    />,
  );
  expect(screen.getByText('驾驶舱')).toBeInTheDocument();
  expect(screen.getByText(/副本代次 243/)).toBeInTheDocument();
  const list = screen.getByRole('list');
  expect(within(list).getAllByRole('listitem')).toHaveLength(12);
  expect(container.querySelectorAll('.ui-title-block__filler')).toHaveLength(0);
  expect(
    (container.firstElementChild as HTMLElement).style.getPropertyValue(
      '--ui-title-columns',
    ),
  ).toBe('4');
});

it('标红的格子带红色方块，不单靠颜色', () => {
  render(<TitleBlock title="驾驶舱" cells={twelve} />);
  const slow = cellOf('每轮同步');
  expect(slow).toHaveClass('ui-title-block__cell--signal');
  expect(slow.querySelector('.ui-signal-mark')).toHaveAttribute(
    'aria-hidden',
    'true',
  );
  expect(slow).toHaveAttribute('title', '超过时限的七成');
  expect(cellOf('副本新鲜度').querySelector('.ui-signal-mark')).toBeNull();
});

it('最后一行不满时补空白格，空白格不进无障碍树', () => {
  const { container } = render(
    <TitleBlock title="项目" cells={twelve.slice(0, 5)} />,
  );
  const fillers = container.querySelectorAll('.ui-title-block__filler');
  expect(fillers).toHaveLength(1);
  expect(fillers[0]).toHaveAttribute('aria-hidden', 'true');
  expect(
    within(screen.getByRole('list')).getAllByRole('listitem'),
  ).toHaveLength(5);
  expect(
    (container.firstElementChild as HTMLElement).style.getPropertyValue(
      '--ui-title-columns',
    ),
  ).toBe('2');
});

it('没有读数格时只画图名', () => {
  render(<TitleBlock title="策略" cells={[]} />);
  expect(screen.getByText('策略')).toBeInTheDocument();
  expect(screen.queryByRole('list')).toBeNull();
});
