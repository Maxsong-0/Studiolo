import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { CargoDependencyPanel } from '../components/CargoDependencyPanel';
import { architectureData, architectureLocation } from './architectureFixtures';
import { cargoData, cargoGraph } from './cargoFixtures';
import { operationsData } from './operationsFixtures';

function mount(data = operationsData({ cargo_dependencies: cargoData() })) {
  const onOpen = vi.fn();
  render(<CargoDependencyPanel data={data} onOpen={onOpen} />);
  return onOpen;
}

it('清单方向、循环、自环、目标条件、optional和继承来源均可见，按三种依赖筛选', async () => {
  const user = userEvent.setup();
  const onOpen = mount();
  const edges = screen.getByRole('list', { name: 'Cargo 内部依赖声明' });
  expect(within(edges).getAllByRole('listitem')).toHaveLength(4);
  expect(edges).toHaveTextContent('renamed_core');
  expect(edges).toHaveTextContent('继承定义来源：Cargo.toml · 第 40 行');
  expect(edges).toHaveTextContent('cfg(target_os = "linux")');
  expect(edges).toHaveTextContent('可选依赖（optional）');
  expect(screen.getByText(/外部依赖声明：3 条/)).toHaveTextContent(
    '不是唯一包数量',
  );
  for (const [kind, count] of [
    ['normal', 2],
    ['dev', 1],
    ['build', 1],
  ] as const) {
    await user.selectOptions(
      screen.getByRole('combobox', { name: '清单依赖种类' }),
      kind,
    );
    expect(within(edges).getAllByRole('listitem')).toHaveLength(count);
  }
  await user.click(
    screen.getByRole('button', { name: '选择依赖终点包 tools' }),
  );
  expect(
    screen.getByRole('complementary', { name: '所选 Cargo 包' }),
  ).toHaveTextContent('tools/Cargo.toml · 第 4 行');
  expect(onOpen).not.toHaveBeenCalled();
  expect(
    screen.queryByRole('button', { name: /Cargo\.toml/ }),
  ).not.toBeInTheDocument();
});

it('只按包id精确关联全部架构节点，不从名称或目录前缀猜测，Vault来源仍可打开', async () => {
  const architecture = architectureData();
  architecture.nodes.push({
    ...architecture.nodes[1],
    id: 'core-view',
    label: '索引视图',
    source: architectureLocation(35),
  });
  architecture.nodes[2].label = 'tools';
  const onOpen = mount(
    operationsData({ cargo_dependencies: cargoData(), architecture }),
  );
  const core = screen.getByRole('button', { name: '选择 Cargo 包 core' });
  core.focus();
  await userEvent.keyboard('{Enter}');
  expect(core).toHaveFocus();
  expect(core).toHaveAttribute('aria-pressed', 'true');
  const linked = screen.getByRole('list', { name: '关联的架构节点' });
  expect(within(linked).getAllByRole('listitem')).toHaveLength(2);
  expect(linked).not.toHaveTextContent('深层资料');
  await userEvent.click(
    within(linked).getByRole('button', {
      name: '索引视图，查看关联的 Vault 声明',
    }),
  );
  expect(onOpen).toHaveBeenCalledWith(architectureLocation(35));
  await userEvent.click(
    screen.getByRole('button', { name: '选择 Cargo 包 tools' }),
  );
  expect(
    screen.queryByRole('list', { name: '关联的架构节点' }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText('没有目录与此包 ID 精确匹配的架构节点。'),
  ).toBeInTheDocument();
});

it('条件与诊断保持纯文本，不渲染HTML或执行操作', () => {
  const graph = cargoGraph();
  graph.dependencies[0].target = '<img src=x onerror="alert(1)">';
  mount(
    operationsData({
      cargo_dependencies: cargoData({
        graph,
        diagnostics: ['<script>alert(2)</script>'],
      }),
    }),
  );
  expect(
    screen.getByText('<img src=x onerror="alert(1)">'),
  ).toBeInTheDocument();
  expect(screen.getByText('<script>alert(2)</script>')).toBeInTheDocument();
  expect(document.querySelector('img, script')).toBeNull();
  expect(screen.getByText(/不代表源码调用或实际构建图/)).toBeInTheDocument();
});

it.each([
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
] as const)('%s状态独立降级，不能展示旧图或观测时间', (status) => {
  mount(
    operationsData({
      cargo_dependencies: cargoData({
        status,
        graph: null,
        observed_at: null,
        diagnostics: ['本次没有清单图'],
      }),
    }),
  );
  expect(screen.getByRole('status')).toBeInTheDocument();
  expect(
    screen.queryByRole('group', { name: 'Cargo 包' }),
  ).not.toBeInTheDocument();
  expect(document.querySelector('time')).toBeNull();
  expect(screen.getByText('本次没有清单图')).toBeInTheDocument();
});

it('缺字段旧响应和空边有明确解释，时间使用独立仓库观测值', async () => {
  const { rerender } = render(
    <CargoDependencyPanel data={operationsData()} onOpen={vi.fn()} />,
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '当前服务未提供 Cargo 清单依赖观测',
  );
  rerender(
    <CargoDependencyPanel
      data={operationsData({
        cargo_dependencies: cargoData({
          graph: cargoGraph({ dependencies: [] }),
        }),
      })}
      onOpen={vi.fn()}
    />,
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '当前种类没有内部依赖声明',
  );
  expect(document.querySelector('time')).toHaveAttribute(
    'dateTime',
    new Date(1789140000 * 1000).toISOString(),
  );
});

it('包与依赖默认只铺前12行、展开后一条不少，折起时所选包仍接在后面可见', async () => {
  const user = userEvent.setup();
  const packages = Array.from({ length: 14 }, (_, index) => ({
    id: `crates/p${index}`,
    name: `p${index}`,
    source: { path: `crates/p${index}/Cargo.toml`, line: 2 },
  }));
  const dependencies = Array.from({ length: 15 }, (_, index) => ({
    from: packages[index % 14].id,
    to: packages[(index + 1) % 14].id,
    alias: `p${(index + 1) % 14}`,
    kind: 'normal' as const,
    target: null,
    optional: false,
    source: { path: `${packages[index % 14].id}/Cargo.toml`, line: 10 + index },
    inherited_from: null,
  }));
  mount(
    operationsData({
      cargo_dependencies: cargoData({
        graph: cargoGraph({ packages, dependencies }),
      }),
    }),
  );
  const group = screen.getByRole('group', { name: 'Cargo 包' });
  const edges = screen.getByRole('list', { name: 'Cargo 内部依赖声明' });
  expect(within(group).getAllByRole('button')).toHaveLength(12);
  expect(within(edges).getAllByRole('listitem')).toHaveLength(12);
  expect(screen.getByText('15 条内部依赖声明')).toBeInTheDocument();
  await user.click(
    within(edges).getByRole('button', { name: '选择依赖终点包 p12' }),
  );
  expect(within(group).getAllByRole('button')).toHaveLength(13);
  expect(
    within(group).getByRole('button', { name: '选择 Cargo 包 p12' }),
  ).toHaveAttribute('aria-pressed', 'true');
  expect(
    screen.getByRole('complementary', { name: '所选 Cargo 包' }),
  ).toHaveTextContent('crates/p12/Cargo.toml · 第 2 行');
  const allPackages = screen.getByRole('button', { name: '显示全部 14 个包' });
  expect(allPackages).toHaveAttribute('aria-expanded', 'false');
  await user.click(allPackages);
  expect(within(group).getAllByRole('button')).toHaveLength(14);
  expect(allPackages).toHaveAttribute('aria-expanded', 'true');
  expect(allPackages).toHaveAccessibleName('收起，只显示前 12 个包');
  await user.click(
    screen.getByRole('button', { name: '显示全部 15 条内部依赖声明' }),
  );
  expect(within(edges).getAllByRole('listitem')).toHaveLength(15);
  expect(edges).toHaveTextContent(
    '依赖引用来源：crates/p0/Cargo.toml · 第 24 行',
  );
  await user.selectOptions(
    screen.getByRole('combobox', { name: '清单依赖种类' }),
    'dev',
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '当前种类没有内部依赖声明',
  );
  expect(
    screen.queryByRole('button', { name: /条内部依赖声明|只显示前 12 条/ }),
  ).not.toBeInTheDocument();
});
