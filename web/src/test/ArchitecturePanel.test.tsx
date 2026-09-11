import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { ArchitecturePanel } from '../components/ArchitecturePanel';
import {
  architectureData,
  architectureLocation,
  architectureSource,
} from './architectureFixtures';
import { operationsData } from './operationsFixtures';

function mount(data = operationsData({ architecture: architectureData() })) {
  const onOpen = vi.fn();
  render(<ArchitecturePanel data={data} onOpen={onOpen} />);
  return onOpen;
}

it('展示循环与自环的文档关系，筛选类型并从节点、关系及声明回原文', async () => {
  const user = userEvent.setup();
  const onOpen = mount();
  expect(
    screen.getByText('关系来自文档声明，不代表实测调用或已实现的数据流。'),
  ).toBeInTheDocument();
  expect(
    within(screen.getByRole('list', { name: '架构关系声明' })).getAllByRole(
      'listitem',
    ),
  ).toHaveLength(4);
  expect(screen.getByText('索引反馈自环')).toBeInTheDocument();
  await user.click(
    screen.getByRole('button', { name: '选择架构节点 资料索引' }),
  );
  await user.click(screen.getByRole('button', { name: '查看节点声明原文' }));
  expect(onOpen).toHaveBeenLastCalledWith(architectureLocation(8));
  await user.selectOptions(
    screen.getByRole('combobox', { name: '关系类型' }),
    'data_flow',
  );
  expect(
    within(screen.getByRole('list', { name: '架构关系声明' })).getAllByRole(
      'listitem',
    ),
  ).toHaveLength(2);
  await user.click(
    screen.getByRole('button', {
      name: '资料索引到检索接口，查看数据流声明来源',
    }),
  );
  expect(onOpen).toHaveBeenLastCalledWith(architectureLocation(17));
  await user.click(screen.getByRole('button', { name: '查看架构声明' }));
  expect(onOpen).toHaveBeenLastCalledWith(architectureSource);
});

it.each([
  ['检索接口', '已声明 · 目录缺失'],
  ['资料索引', '已声明 · 目录存在'],
  ['导出工具', '目录存在 · 未声明'],
  ['阅读者', '未核验'],
  ['深层资料', '未核验'],
])('节点%s只使用目录精确匹配的证据', async (label, status) => {
  mount();
  await userEvent.click(
    screen.getByRole('button', { name: `选择架构节点 ${label}` }),
  );
  expect(
    screen.getByRole('complementary', { name: '所选架构节点' }),
  ).toHaveTextContent(status);
});

it('仓库未绑定时不能把声明目录的旧状态当作已核验', async () => {
  mount(
    operationsData({
      architecture: architectureData(),
      repository: { status: 'unbound', observed_at: null },
    }),
  );
  await userEvent.click(
    screen.getByRole('button', { name: '选择架构节点 资料索引' }),
  );
  expect(screen.getByRole('complementary')).toHaveTextContent('未核验');
  expect(screen.getByRole('complementary')).not.toHaveTextContent(
    '已声明 · 目录存在',
  );
});

it('设计步骤手动推进、回退及选择，允许同一节点再次出现，并保留切换前状态', async () => {
  const user = userEvent.setup();
  const onOpen = mount();
  await user.click(screen.getByRole('button', { name: '设计路径' }));
  expect(screen.getByText(/下列步骤仅表示文档声明的设计顺序/)).toBeVisible();
  expect(screen.getByText('设计顺序 · 第 1 / 3 步')).toBeVisible();
  const next = screen.getByRole('button', { name: '后一步' });
  next.focus();
  await user.keyboard('{Enter}');
  expect(screen.getByText('设计顺序 · 第 2 / 3 步')).toBeVisible();
  expect(next).toHaveFocus();
  await user.click(screen.getByRole('button', { name: '查看步骤声明原文' }));
  expect(onOpen).toHaveBeenLastCalledWith(architectureLocation(25));
  await user.click(next);
  expect(screen.getByText('设计顺序 · 第 3 / 3 步')).toBeVisible();
  expect(next).toHaveAttribute('aria-disabled', 'true');
  await user.click(next);
  expect(screen.getByText('设计顺序 · 第 3 / 3 步')).toBeVisible();
  await user.click(screen.getByRole('button', { name: '前一步' }));
  await user.click(screen.getByRole('button', { name: '模块关系' }));
  await user.click(screen.getByRole('button', { name: '设计路径' }));
  expect(screen.getByText('设计顺序 · 第 2 / 3 步')).toBeVisible();
  await user.selectOptions(
    screen.getByRole('combobox', { name: '设计路径' }),
    'export',
  );
  expect(screen.getByText('设计顺序 · 第 1 / 2 步')).toBeVisible();
  await user.click(
    within(screen.getByRole('list', { name: '设计步骤' })).getByRole('button', {
      name: '2 导出工具',
    }),
  );
  expect(screen.getByText('生成分享资料')).toBeVisible();
});

it('标签、说明和诊断按文本显示，键盘可选择节点', async () => {
  const hostile = '<img src=x onerror="alert(1)">模块';
  const architecture = architectureData();
  architecture.nodes[0] = {
    ...architecture.nodes[0],
    label: hostile,
    description: '<svg onload="alert(2)">说明',
  };
  architecture.diagnostics = ['<script>alert(3)</script>'];
  mount(operationsData({ architecture }));
  const node = screen.getByRole('button', { name: `选择架构节点 ${hostile}` });
  node.focus();
  await userEvent.keyboard('{Enter}');
  expect(node).toHaveAttribute('aria-pressed', 'true');
  expect(screen.getByRole('complementary')).toHaveTextContent(
    '<svg onload="alert(2)">说明',
  );
  expect(screen.getByText('<script>alert(3)</script>')).toBeInTheDocument();
  expect(document.querySelector('img, script, svg[onload]')).toBeNull();
});

it.each(['unavailable', 'invalid'] as const)(
  '%s声明保留说明和原文入口',
  async (status) => {
    const onOpen = mount(
      operationsData({
        architecture: architectureData({
          status,
          nodes: [],
          relations: [],
          flows: [],
          diagnostics: ['缺少节点表'],
        }),
      }),
    );
    expect(screen.getByRole('status')).toHaveTextContent(
      '此项目尚无可展示的模块关系与设计路径',
    );
    expect(
      screen.queryByRole('group', { name: '架构节点' }),
    ).not.toBeInTheDocument();
    expect(screen.getByText('缺少节点表')).toBeInTheDocument();
    await userEvent.click(screen.getByRole('button', { name: '查看架构声明' }));
    expect(onOpen).toHaveBeenCalledWith(architectureSource);
  },
);

it('旧响应缺架构字段时显示不可用，空关系和空路径也有解释', async () => {
  const { rerender } = render(
    <ArchitecturePanel data={operationsData()} onOpen={vi.fn()} />,
  );
  expect(screen.getByRole('status')).toHaveTextContent('此项目尚无可展示');
  rerender(
    <ArchitecturePanel
      data={operationsData({
        architecture: architectureData({ relations: [], flows: [] }),
      })}
      onOpen={vi.fn()}
    />,
  );
  expect(screen.getByText('当前类型没有关系声明。')).toBeVisible();
  await userEvent.click(screen.getByRole('button', { name: '设计路径' }));
  expect(
    screen.getByText('文档尚未声明设计路径。已有模块关系仍可查看。'),
  ).toBeVisible();
});

it('设计步骤沿用节点的精确目录证据，缺失与未观测目录不会被流程顺序掩盖', async () => {
  const architecture = architectureData();
  architecture.flows[0].steps[1].node = 'detail';
  mount(operationsData({ architecture }));
  await userEvent.click(screen.getByRole('button', { name: '设计路径' }));
  expect(screen.getByLabelText('当前设计步骤')).toHaveTextContent(
    '目录：web · 已声明 · 目录缺失',
  );
  await userEvent.click(screen.getByRole('button', { name: '后一步' }));
  expect(screen.getByLabelText('当前设计步骤')).toHaveTextContent(
    '目录：crates/core/internal · 未核验',
  );
});
