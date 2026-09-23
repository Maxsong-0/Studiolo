import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import type { ArchitectureData } from '../architecture';
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

/* ---------- 关系图几何：黑线工程图的连线只走正交线，不穿过任何节点框，箭头顶在终点框边上 ---------- */

type Point = [number, number];
interface Box {
  label: string;
  x: number;
  y: number;
  width: number;
  height: number;
}

const pointsOf = (path: string): Point[] =>
  [...path.matchAll(/[ML](-?[\d.]+) (-?[\d.]+)/g)].map((match) => [
    Number(match[1]),
    Number(match[2]),
  ]);

function readDrawing() {
  const canvas = screen.getByRole('group', { name: '架构节点' });
  const width = parseFloat(
    canvas.style.getPropertyValue('--architecture-node-width'),
  );
  const height = parseFloat(
    canvas.style.getPropertyValue('--architecture-node-height'),
  );
  const boxes: Box[] = within(canvas)
    .getAllByRole('button')
    .map((button) => ({
      label: button.getAttribute('aria-label')!.replace('选择架构节点 ', ''),
      x: parseFloat(button.style.left),
      y: parseFloat(button.style.top),
      width,
      height,
    }));
  const wires = [...canvas.querySelectorAll('.architecture-wire')].map(
    (wire) => ({
      className: wire.getAttribute('class')!,
      line: pointsOf(
        wire.querySelector('.architecture-wire__line')!.getAttribute('d')!,
      ),
      head: pointsOf(
        wire.querySelector('.architecture-wire__head')!.getAttribute('d')!,
      ),
    }),
  );
  return { boxes, wires };
}

/** 线段是否穿进框内（压在框边上不算）。 */
function crossesBox([x0, y0]: Point, [x1, y1]: Point, box: Box) {
  const [left, right] = [box.x + 0.5, box.x + box.width - 0.5];
  const [top, bottom] = [box.y + 0.5, box.y + box.height - 0.5];
  if (Math.abs(y0 - y1) < 1)
    return (
      y0 > top &&
      y0 < bottom &&
      Math.max(x0, x1) > left &&
      Math.min(x0, x1) < right
    );
  return (
    x0 > left &&
    x0 < right &&
    Math.max(y0, y1) > top &&
    Math.min(y0, y1) < bottom
  );
}

function expectCleanDrawing(architecture: ArchitectureData) {
  const { boxes, wires } = readDrawing();
  const labels = new Map(
    architecture.nodes.map((node) => [node.id, node.label]),
  );
  const boxOf = (id: string) =>
    boxes.find((box) => box.label === labels.get(id))!;
  expect(boxes).toHaveLength(architecture.nodes.length);
  expect(boxes.every((box) => box.width > 0 && box.height > 0)).toBe(true);
  boxes.forEach((box, index) =>
    boxes
      .slice(index + 1)
      .forEach((other) =>
        expect(
          box.x + box.width <= other.x ||
            other.x + other.width <= box.x ||
            box.y + box.height <= other.y ||
            other.y + other.height <= box.y,
        ).toBe(true),
      ),
  );
  expect(wires).toHaveLength(architecture.relations.length);
  architecture.relations.forEach((relation, index) => {
    const { line, head, className } = wires[index];
    const [from, to] = [boxOf(relation.from), boxOf(relation.to)];
    expect(className).toContain(
      relation.kind === 'data_flow' ? 'is-data-flow' : 'is-dependency',
    );
    expect(
      [...line, ...head].every(
        ([x, y]) => Number.isFinite(x) && Number.isFinite(y),
      ),
    ).toBe(true);
    for (let step = 1; step < line.length; step += 1) {
      const [[x0, y0], [x1, y1]] = [line[step - 1], line[step]];
      expect(x0 === x1 || Math.abs(y0 - y1) < 1).toBe(true);
      for (const box of boxes)
        expect(crossesBox(line[step - 1], line[step], box)).toBe(false);
    }
    const [tipX, tipY] = head[0];
    const [endX, endY] = line[line.length - 1];
    if (relation.from === relation.to) {
      expect(tipY).toBe(to.y);
      expect(tipX > to.x && tipX < to.x + to.width).toBe(true);
      expect(endX).toBe(tipX);
      expect(tipY - endY).toBeCloseTo(7, 5);
      return;
    }
    expect([to.x, to.x + to.width]).toContain(tipX);
    expect(tipY > to.y && tipY < to.y + to.height).toBe(true);
    expect(Math.abs(endX - tipX)).toBeCloseTo(7, 5);
    expect(endY).toBeCloseTo(tipY, 5);
    const [startX, startY] = line[0];
    expect([from.x, from.x + from.width]).toContain(startX);
    expect(startY > from.y && startY < from.y + from.height).toBe(true);
  });
  return { boxes, wires, boxOf };
}

it('关系图按原方向画出依赖与数据流：环里的回边箭头朝左，自环在框上沿，筛选只减线不挪节点', async () => {
  const user = userEvent.setup();
  const architecture = architectureData();
  mount(operationsData({ architecture }));
  const { boxes, wires, boxOf } = expectCleanDrawing(architecture);
  // web → core 是依赖；core → web 与之成环，箭头回到 web 的右边
  expect(wires[0].head[0][0]).toBe(boxOf('core').x);
  expect(wires[1].head[0][0]).toBe(boxOf('web').x + boxOf('web').width);
  expect(boxOf('core').x).toBeGreaterThan(boxOf('web').x);
  expect(boxOf('export').x).toBeGreaterThan(boxOf('core').x);
  // 默认选中第一个节点：与它相连的两条线加粗，其余不加粗
  expect(wires.map((wire) => wire.className.includes('is-active'))).toEqual([
    true,
    true,
    false,
    false,
  ]);
  await user.selectOptions(
    screen.getByRole('combobox', { name: '关系类型' }),
    'data_flow',
  );
  const filtered = readDrawing();
  expect(filtered.boxes).toEqual(boxes);
  expect(filtered.wires.map((wire) => wire.className)).toEqual([
    'architecture-wire is-data-flow is-active',
    'architecture-wire is-data-flow',
  ]);
});

it('没有关系声明时节点按声明顺序铺成几列，不堆成一整列', () => {
  const architecture = architectureData({ relations: [], flows: [] });
  mount(operationsData({ architecture }));
  const { boxes } = expectCleanDrawing(architecture);
  const columns = [...new Set(boxes.map((box) => box.x))];
  expect(columns.length).toBeGreaterThan(1);
  // 节点框按声明顺序读出：同一列里从上到下仍是声明顺序
  for (const x of columns) {
    const heights = boxes.filter((box) => box.x === x).map((box) => box.y);
    expect(heights).toEqual([...heights].sort((left, right) => left - right));
  }
});

it('稠密的声明（环、长跨层、双向、同对两类、自环）照样画成干净的正交图', () => {
  // mulberry32：固定种子，每次跑出同一张图
  let seed = 20260922;
  const random = () => {
    seed = (seed + 0x6d2b79f5) | 0;
    let value = Math.imul(seed ^ (seed >>> 15), 1 | seed);
    value = (value + Math.imul(value ^ (value >>> 7), 61 | value)) ^ value;
    return ((value ^ (value >>> 14)) >>> 0) / 4294967296;
  };
  const nodes = Array.from({ length: 14 }, (_, index) => ({
    id: `n${index}`,
    label: `节点${index}`,
    kind: 'module' as const,
    directory: null,
    description: '说明',
    source: architectureLocation(10 + index),
  }));
  const relations: ArchitectureData['relations'] = [];
  const seen = new Set<string>();
  const add = (from: string, to: string, kind: 'dependency' | 'data_flow') => {
    if (seen.has(`${from}:${to}:${kind}`)) return;
    seen.add(`${from}:${to}:${kind}`);
    relations.push({
      from,
      to,
      kind,
      label: `${from}到${to}`,
      source: architectureLocation(40 + relations.length),
    });
  };
  // 先放必有的难点：长链与跨四层的直达、双向、同一对的两类关系、两个自环、三角环
  ['n0', 'n1', 'n2', 'n3', 'n4'].forEach((id, index, chain) => {
    if (index) add(chain[index - 1], id, 'dependency');
  });
  add('n0', 'n4', 'dependency');
  add('n4', 'n0', 'data_flow');
  add('n5', 'n6', 'dependency');
  add('n5', 'n6', 'data_flow');
  add('n6', 'n6', 'data_flow');
  add('n6', 'n6', 'dependency');
  add('n7', 'n8', 'dependency');
  add('n8', 'n9', 'dependency');
  add('n9', 'n7', 'data_flow');
  while (relations.length < 40)
    add(
      `n${Math.floor(random() * nodes.length)}`,
      `n${Math.floor(random() * nodes.length)}`,
      random() < 0.5 ? 'dependency' : 'data_flow',
    );
  const architecture = architectureData({ nodes, relations, flows: [] });
  mount(operationsData({ architecture }));
  const { boxOf } = expectCleanDrawing(architecture);
  // 链上的节点从左到右依次排开
  for (const [left, right] of [
    ['n0', 'n1'],
    ['n1', 'n2'],
    ['n2', 'n3'],
    ['n3', 'n4'],
  ])
    expect(boxOf(right).x).toBeGreaterThan(boxOf(left).x);
});
