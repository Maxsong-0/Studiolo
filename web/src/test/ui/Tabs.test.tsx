import { useState } from 'react';
import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { Tabs, type TabItem } from '../../ui';

const items: TabItem[] = [
  { id: 'topology', label: '拓扑' },
  { id: 'board', label: '任务与回执', count: 12 },
  { id: 'timeline', label: '时间线' },
  { id: 'execution', label: '执行' },
];

function Harness({ onChange }: { onChange?: (id: string) => void }) {
  const [value, setValue] = useState('topology');
  return (
    <Tabs
      items={items}
      value={value}
      onChange={(id) => {
        onChange?.(id);
        setValue(id);
      }}
      ariaLabel="协作子标签"
      idPrefix="collab"
    />
  );
}

const selectedNames = () =>
  screen
    .getAllByRole('tab')
    .filter((tab) => tab.getAttribute('aria-selected') === 'true')
    .map((tab) => tab.textContent);

it('渲染 tablist，只有当前项 aria-selected 为 true 且在 Tab 序列里', () => {
  render(<Harness />);
  const list = screen.getByRole('tablist', { name: '协作子标签' });
  expect(list).toBeInTheDocument();
  const tabs = screen.getAllByRole('tab');
  expect(tabs).toHaveLength(4);
  expect(tabs.map((tab) => tab.getAttribute('aria-selected'))).toEqual([
    'true',
    'false',
    'false',
    'false',
  ]);
  expect(tabs.map((tab) => tab.tabIndex)).toEqual([0, -1, -1, -1]);
  expect(screen.getByRole('tab', { name: '任务与回执 12' })).toHaveAttribute(
    'aria-controls',
    'collab-panel-board',
  );
  expect(tabs[0]).toHaveAttribute('id', 'collab-tab-topology');
});

it('左右方向键切换并移动焦点，首尾循环；Home/End 到首尾', async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Harness onChange={onChange} />);
  await user.tab();
  expect(screen.getByRole('tab', { name: '拓扑' })).toHaveFocus();

  await user.keyboard('{ArrowRight}');
  expect(onChange).toHaveBeenLastCalledWith('board');
  expect(selectedNames()).toEqual(['任务与回执 12']);
  expect(screen.getByRole('tab', { name: '任务与回执 12' })).toHaveFocus();
  expect(screen.getByRole('tab', { name: '任务与回执 12' }).tabIndex).toBe(0);

  await user.keyboard('{End}');
  expect(selectedNames()).toEqual(['执行']);
  expect(screen.getByRole('tab', { name: '执行' })).toHaveFocus();

  await user.keyboard('{ArrowRight}');
  expect(selectedNames()).toEqual(['拓扑']);
  expect(screen.getByRole('tab', { name: '拓扑' })).toHaveFocus();

  await user.keyboard('{ArrowLeft}');
  expect(selectedNames()).toEqual(['执行']);

  await user.keyboard('{Home}');
  expect(selectedNames()).toEqual(['拓扑']);
  expect(onChange.mock.calls.map(([id]) => id)).toEqual([
    'board',
    'execution',
    'topology',
    'execution',
    'topology',
  ]);
});

it('点击切换；点当前项不重复触发 onChange，其他键不拦截', async () => {
  const user = userEvent.setup();
  const onChange = vi.fn();
  render(<Harness onChange={onChange} />);
  await user.click(screen.getByRole('tab', { name: '拓扑' }));
  expect(onChange).not.toHaveBeenCalled();
  await user.click(screen.getByRole('tab', { name: '时间线' }));
  expect(onChange).toHaveBeenCalledWith('timeline');
  expect(selectedNames()).toEqual(['时间线']);
  await user.keyboard('{ArrowDown}');
  expect(onChange).toHaveBeenCalledTimes(1);
});

it('value 不在 items 里时第一项进入 Tab 序列，没有项被选中', () => {
  render(
    <Tabs
      items={items}
      value="missing"
      onChange={() => {}}
      ariaLabel="子标签"
    />,
  );
  const tabs = screen.getAllByRole('tab');
  expect(tabs.map((tab) => tab.tabIndex)).toEqual([0, -1, -1, -1]);
  expect(selectedNames()).toEqual([]);
  expect(tabs[0]).not.toHaveAttribute('aria-controls');
});
