import { createRef } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  GridFrame,
  KeyHint,
  Skeleton,
  Stat,
  Surface,
} from '../../ui';

it('Button 缺省 type=button、secondary；变体与尺寸落到类名，ref 直达原生按钮', async () => {
  const user = userEvent.setup();
  const onClick = vi.fn();
  const ref = createRef<HTMLButtonElement>();
  render(
    <form onSubmit={() => onClick('submit')}>
      <Button ref={ref} onClick={() => onClick('click')}>
        打开任务书
      </Button>
      <Button variant="danger" size="sm" disabled>
        释放写域
      </Button>
    </form>,
  );
  const open = screen.getByRole('button', { name: '打开任务书' });
  expect(open).toHaveAttribute('type', 'button');
  expect(open).toHaveClass(
    'ui-button',
    'ui-button--secondary',
    'ui-button--md',
  );
  expect(ref.current).toBe(open);
  await user.click(open);
  expect(onClick.mock.calls).toEqual([['click']]);
  const release = screen.getByRole('button', { name: '释放写域' });
  expect(release).toHaveClass('ui-button--danger', 'ui-button--sm');
  expect(release).toBeDisabled();
});

it('ErrorState 是 alert，重试按钮名字可换；EmptyState 只是朴素文字', async () => {
  const user = userEvent.setup();
  const onRetry = vi.fn();
  render(
    <>
      <ErrorState title="目录读取失败" onRetry={onRetry} retryLabel="重试目录">
        本机视图暂不可用。
      </ErrorState>
      <EmptyState title="黑板未启用" action={<Button>去设置</Button>}>
        这台机器没有接入协作黑板。
      </EmptyState>
    </>,
  );
  const alert = screen.getByRole('alert');
  expect(alert).toHaveTextContent('目录读取失败');
  expect(alert).toHaveTextContent('本机视图暂不可用。');
  await user.click(screen.getByRole('button', { name: '重试目录' }));
  expect(onRetry).toHaveBeenCalledTimes(1);
  expect(screen.getByText('黑板未启用')).toBeInTheDocument();
  expect(screen.getByText('这台机器没有接入协作黑板。')).toBeInTheDocument();
  expect(screen.getAllByRole('alert')).toHaveLength(1);
});

it('ErrorState 不给 onRetry 就没有按钮', () => {
  render(<ErrorState title="读取失败" />);
  expect(screen.getByRole('alert')).toHaveTextContent('读取失败');
  expect(screen.queryByRole('button')).toBeNull();
});

it('Skeleton 外层 role=status 报告读取中，斜线纹条不进无障碍树', () => {
  const { container, rerender } = render(<Skeleton />);
  const status = screen.getByRole('status');
  expect(status).toHaveTextContent('正在读取…');
  const lines = container.querySelector('.ui-skeleton__lines');
  expect(lines).toHaveAttribute('aria-hidden', 'true');
  expect(container.querySelectorAll('.ui-skeleton__line')).toHaveLength(3);
  rerender(<Skeleton lines={5} label="正在读取目录" />);
  expect(screen.getByRole('status')).toHaveTextContent('正在读取目录');
  expect(container.querySelectorAll('.ui-skeleton__line')).toHaveLength(5);
});

it('Stat 缺值显示「—」并把原因放进 title；标红时带红色方块', () => {
  const { container, rerender } = render(
    <Stat label="副本新鲜度" value={null} unit="s" reason="副本代次读取失败" />,
  );
  const root = container.firstElementChild as HTMLElement;
  expect(screen.getByText('—')).toBeInTheDocument();
  expect(root).toHaveAttribute('title', '副本代次读取失败');
  // 缺值时不挂单位，免得读成「— s」
  expect(root.querySelector('.ui-stat__unit')).toBeNull();
  expect(screen.getByText('副本代次读取失败')).toHaveClass('sr-only');

  rerender(<Stat label="每轮同步" value={85} unit="s" tone="signal" />);
  expect(screen.getByText('85')).toBeInTheDocument();
  expect(screen.getByText('s')).toHaveClass('ui-stat__unit');
  expect(root).toHaveClass('ui-stat--signal');
  expect(root.querySelector('.ui-signal-mark')).toHaveAttribute(
    'aria-hidden',
    'true',
  );
  expect(root).not.toHaveAttribute('title');
});

it('GridFrame 与 Cell 把列数、跨度交给 CSS 变量；缺省跨度占满整行', () => {
  const { container } = render(
    <GridFrame rows="300px minmax(0, 1fr) 190px" aria-label="驾驶舱">
      <Cell span={3} rowSpan={2}>
        <CellHeader title="待我动作" meta="全部 5 项" />
      </Cell>
      <Cell surface="grid" as="div" flush>
        拓扑
      </Cell>
    </GridFrame>,
  );
  const frame = container.firstElementChild as HTMLElement;
  expect(frame).toHaveClass('ui-grid');
  expect(frame).toHaveAttribute('aria-label', '驾驶舱');
  expect(frame.style.getPropertyValue('--ui-grid-columns')).toBe('12');
  expect(frame.style.getPropertyValue('--ui-grid-rows')).toBe(
    '300px minmax(0, 1fr) 190px',
  );
  const [first, second] = Array.from(frame.children) as HTMLElement[];
  expect(first.tagName).toBe('SECTION');
  expect(first.style.getPropertyValue('--ui-cell-column')).toBe('span 3');
  expect(first.style.getPropertyValue('--ui-cell-row')).toBe('span 2');
  expect(
    screen.getByRole('heading', { level: 2, name: '待我动作' }),
  ).toBeInTheDocument();
  expect(screen.getByText('全部 5 项')).toBeInTheDocument();
  expect(second.tagName).toBe('DIV');
  expect(second).toHaveClass('ui-cell--grid', 'ui-cell--flush');
  expect(second.style.getPropertyValue('--ui-cell-column')).toBe('');
});

it('KeyHint 用嵌套 kbd 表示组合键；Surface 可换元素并转交 ref', () => {
  const ref = createRef<HTMLDialogElement>();
  const { container } = render(
    <>
      <KeyHint keys={['⌘', 'K']} />
      <Surface as="dialog" ref={ref} aria-label="动作面板" appear={false}>
        面板
      </Surface>
    </>,
  );
  const hint = container.querySelector('kbd.ui-keyhint');
  expect(hint).not.toBeNull();
  expect(
    Array.from(hint!.querySelectorAll('kbd')).map((key) => key.textContent),
  ).toEqual(['⌘', 'K']);
  expect(ref.current).toBeInstanceOf(HTMLDialogElement);
  expect(ref.current).toHaveClass('ui-surface');
  expect(ref.current).not.toHaveClass('ui-surface--appear');
  expect(ref.current).toHaveAttribute('aria-label', '动作面板');
});
