import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RustCallSitesPanel } from '../components/RustCallSitesPanel';
import { architectureData, architectureLocation } from './architectureFixtures';
import { operationsData } from './operationsFixtures';
import { rustData, rustGraph } from './rustCallFixtures';

function mount(data = operationsData({ rust_call_sites: rustData() })) {
  const onOpen = vi.fn();
  render(<RustCallSitesPanel data={data} onOpen={onOpen} />);
  return onOpen;
}

it('按文件、最近词法上下文和调用种类查看，不将闭包内部归入外层即时执行', async () => {
  const user = userEvent.setup();
  const onOpen = mount();
  const calls = () => screen.getByRole('list', { name: 'Rust 调用位置' });
  expect(within(calls()).getAllByRole('listitem')).toHaveLength(4);
  expect(calls()).toHaveTextContent('第 3 行 · UTF-8 字节列 13');
  const contexts = screen.getByRole('group', { name: 'Rust 词法上下文' });
  const outer = within(contexts).getByRole('button', {
    name: /函数 · 处理资料/,
  });
  outer.focus();
  await user.keyboard('{Enter}');
  expect(outer).toHaveFocus();
  expect(within(calls()).getAllByRole('listitem')).toHaveLength(3);
  expect(calls()).not.toHaveTextContent('表达式目标未提取');
  await user.click(within(contexts).getByRole('button', { name: /闭包/ }));
  expect(within(calls()).getAllByRole('listitem')).toHaveLength(1);
  expect(calls()).toHaveTextContent('表达式目标未提取');
  await user.selectOptions(
    screen.getByRole('combobox', { name: '调用语法种类' }),
    'path',
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '当前上下文与种类没有匹配位置',
  );
  await user.click(
    within(contexts).getByRole('button', { name: '全部上下文' }),
  );
  for (const kind of ['path', 'method', 'expression', 'macro']) {
    await user.selectOptions(
      screen.getByRole('combobox', { name: '调用语法种类' }),
      kind,
    );
    expect(within(calls()).getAllByRole('listitem')).toHaveLength(1);
    expect(calls()).toHaveTextContent('目标未解析');
  }
  await user.click(
    screen.getByRole('button', { name: '选择 Rust 文件 src/empty.rs' }),
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '此白名单文件没有提取到调用位置',
  );
  expect(screen.getByRole('combobox', { name: '调用语法种类' })).toHaveValue(
    'all',
  );
  expect(
    screen.queryByRole('button', { name: /函数 · 处理资料/ }),
  ).not.toBeInTheDocument();
  expect(onOpen).not.toHaveBeenCalled();
});

it('全部目录前缀匹配均显示，近似前缀和空目录不关联，只有Vault按钮打开原文', async () => {
  const architecture = architectureData();
  architecture.nodes.push({
    ...architecture.nodes[1],
    id: 'second',
    label: '第二份职责',
    source: architectureLocation(36),
  });
  architecture.nodes[0].directory = 'crates/cor';
  const onOpen = mount(
    operationsData({ rust_call_sites: rustData(), architecture }),
  );
  const linked = screen.getByRole('list', { name: '目录包含的全部架构声明' });
  expect(within(linked).getAllByRole('listitem')).toHaveLength(3);
  expect(linked).toHaveTextContent('深层资料');
  expect(linked).not.toHaveTextContent('检索接口');
  expect(linked).not.toHaveTextContent('阅读者');
  expect(screen.getByText(/仅表示文件位于声明目录下/)).toBeInTheDocument();
  await userEvent.click(
    within(linked).getByRole('button', {
      name: '第二份职责，查看所在目录的 Vault 声明',
    }),
  );
  expect(onOpen).toHaveBeenCalledWith(architectureLocation(36));
  expect(
    screen.queryByRole('button', { name: /仓库源码/ }),
  ).not.toBeInTheDocument();
});

it('显示覆盖、宏和cfg遗漏边界，源码使用独立时间，名称和诊断均为纯文本', () => {
  const graph = rustGraph();
  graph.calls[0].name = '<img src=x onerror="alert(1)">';
  mount(
    operationsData({
      rust_call_sites: rustData({
        graph,
        diagnostics: ['<script>失败说明</script>'],
      }),
    }),
  );
  expect(
    screen.getByText('<img src=x onerror="alert(1)">'),
  ).toBeInTheDocument();
  expect(screen.getByText('<script>失败说明</script>')).toBeInTheDocument();
  expect(document.querySelector('img, script, svg')).toBeNull();
  expect(screen.getByText(/cfg \/ cfg_attr 未求值/)).toHaveTextContent(
    '外部模块未跟随读取',
  );
  expect(screen.getByText('其中条件属性').nextElementSibling).toHaveTextContent(
    '2',
  );
  expect(screen.getByText('宏定义').nextElementSibling).toHaveTextContent('1');
  expect(document.querySelector('time')).toHaveAttribute(
    'datetime',
    new Date(1789140200 * 1000).toISOString(),
  );
});

it.each([
  'disabled',
  'unbound',
  'unavailable',
  'invalid',
  'unsupported',
] as const)('%s不保留图或观测时间', (status) => {
  mount(
    operationsData({
      rust_call_sites: rustData({ status, graph: null, observed_at: null }),
    }),
  );
  expect(screen.getByRole('status')).toBeInTheDocument();
  expect(
    screen.queryByRole('group', { name: 'Rust 白名单文件' }),
  ).not.toBeInTheDocument();
  expect(document.querySelector('time')).toBeNull();
});

it('旧响应明确降级，整批零结果不宣称全仓库没有调用', () => {
  const { rerender } = render(
    <RustCallSitesPanel data={operationsData()} onOpen={vi.fn()} />,
  );
  expect(screen.getByRole('status')).toHaveTextContent(
    '当前服务未提供 Rust 源码调用位置证据',
  );
  const graph = rustGraph({ calls: [] });
  graph.files[0].unexpanded_macros = 0;
  rerender(
    <RustCallSitesPanel
      data={operationsData({ rust_call_sites: rustData({ graph }) })}
      onOpen={vi.fn()}
    />,
  );
  expect(screen.getByText(/零结果只表示白名单中/)).toHaveTextContent(
    '不代表全仓库没有调用',
  );
});
