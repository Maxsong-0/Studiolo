import { useMemo, useState } from 'react';
import { nodeKinds, relationKinds } from '../architecture';
import type {
  ArchitectureData,
  ArchitectureNode,
  ArchitectureRelation,
} from '../architecture';
import type { ProjectOperationsData } from '../projectOperations';
import type { OpenSource } from '../types';
import { Button, Cell } from '../ui';
import { cssVars, cx } from '../ui/dom';
import { ArchitectureNodeDetails } from './ArchitectureNodeDetails';
import { Icon } from './Icon';
import '../styles/projects.css';
import '../styles/architecture.css';

/*
 * 关系图的几何（像素）。节点框定宽定高：名字最多两行，全文在按钮名字、悬停提示与右侧详情里。
 * 从左到右分层，层与层之间是走线通道；跨层的关系从中间层的空位穿过，不压节点框。
 */
const BOX_WIDTH = 156;
const BOX_HEIGHT = 56;
const ROW_GAP = 28;
const PADDING = 16;
/** 通道最窄宽度；通道两侧各留一段给箭头与转角，竖线在中间按条排开。 */
const CHANNEL_MIN = 56;
const CHANNEL_MARGIN = 12;
const TRACK_STEP = 8;
/** 同一侧多条线接同一个框时，接点上下错开的间距；接点离中线最远不超过 PORT_REACH（离框角留 6px）。 */
const PORT_STEP = 9;
const PORT_REACH = BOX_HEIGHT / 2 - 6;
/** 箭头长 7、宽 7。 */
const HEAD = 7;
/** 自环画在框的上沿。 */
const LOOP_HEIGHT = 12;
const LOOP_WIDTH = 22;

interface Wire {
  /** 在 architecture.relations 里的下标。 */
  index: number;
  relation: ArchitectureRelation;
  /** 正交折线（SVG path d），终点停在箭头底边。 */
  line: string;
  /** 实心三角箭头，尖端顶在目标框的边上。 */
  head: string;
}

interface Drawing {
  width: number;
  height: number;
  boxes: Map<string, { x: number; y: number }>;
  wires: Wire[];
}

interface Vertex {
  id: string;
  layer: number;
  seed: number;
  preds: string[];
  succs: string[];
}

const edgeKey = (from: string, to: string) => `${from}>${to}`;

function lowerMedian(values: number[]) {
  const sorted = [...values].sort((left, right) => left - right);
  return sorted[Math.floor((sorted.length - 1) / 2)];
}

const round = (value: number) => Math.round(value * 10) / 10;

function pathOf(points: [number, number][]) {
  return points
    .map(([x, y], index) => `${index ? 'L' : 'M'}${round(x)} ${round(y)}`)
    .join(' ');
}

/**
 * 分层画法（纯函数，只看声明的节点与关系）：
 * 1. 按声明顺序深度优先断环：指回栈上节点的关系只在分层时反向，画出来仍按原方向，箭头朝左；
 * 2. 最长路径分层，跨层关系在中间层插入占位点；
 * 3. 层内顺序从声明顺序起步，上下各扫四遍重心；与谁都不相连的节点最后补进不满的列；
 *    行号尽量与前驱对齐，让直线最多；
 * 4. 每个通道按需要的竖线条数定宽，竖线等距排开；同一侧的多个接点上下错开。
 * 筛选只影响画哪些线，不改布局，切换关系类型时节点不跳。
 */
function layoutDrawing(
  nodes: ArchitectureNode[],
  relations: ArchitectureRelation[],
): Drawing {
  const declared = new Map(nodes.map((node, index) => [node.id, index]));

  const outgoing = new Map<string, string[]>(
    nodes.map((node) => [node.id, []]),
  );
  for (const { from, to } of relations) {
    const targets = outgoing.get(from)!;
    if (from !== to && !targets.includes(to)) targets.push(to);
  }
  const visiting = new Set<string>();
  const visited = new Set<string>();
  const reversed = new Set<string>();
  const visit = (id: string) => {
    visiting.add(id);
    for (const next of outgoing.get(id)!) {
      if (visiting.has(next)) reversed.add(edgeKey(id, next));
      else if (!visited.has(next)) visit(next);
    }
    visiting.delete(id);
    visited.add(id);
  };
  for (const node of nodes) if (!visited.has(node.id)) visit(node.id);
  const oriented = (from: string, to: string): [string, string] =>
    reversed.has(edgeKey(from, to)) ? [to, from] : [from, to];
  const dag = new Map<string, [string, string]>();
  for (const { from, to } of relations)
    if (from !== to) {
      const [head, tail] = oriented(from, to);
      dag.set(edgeKey(head, tail), [head, tail]);
    }

  const layer = new Map<string, number>(nodes.map((node) => [node.id, 0]));
  const next = new Map<string, string[]>(nodes.map((node) => [node.id, []]));
  const waiting = new Map<string, number>(nodes.map((node) => [node.id, 0]));
  for (const [head, tail] of dag.values()) {
    next.get(head)!.push(tail);
    waiting.set(tail, waiting.get(tail)! + 1);
  }
  const queue = nodes
    .filter((node) => waiting.get(node.id) === 0)
    .map((node) => node.id);
  while (queue.length) {
    const id = queue.shift()!;
    for (const target of next.get(id)!) {
      layer.set(target, Math.max(layer.get(target)!, layer.get(id)! + 1));
      waiting.set(target, waiting.get(target)! - 1);
      if (waiting.get(target) === 0) queue.push(target);
    }
  }

  const vertices = new Map<string, Vertex>();
  for (const node of nodes)
    vertices.set(node.id, {
      id: node.id,
      layer: layer.get(node.id)!,
      seed: declared.get(node.id)!,
      preds: [],
      succs: [],
    });
  const chains = new Map<string, string[]>();
  for (const [key, [head, tail]] of dag) {
    const chain = [head];
    for (let step = layer.get(head)! + 1; step < layer.get(tail)!; step += 1) {
      const id = `${key}#${step}`;
      vertices.set(id, {
        id,
        layer: step,
        seed: declared.get(head)! + 0.5,
        preds: [],
        succs: [],
      });
      chain.push(id);
    }
    chain.push(tail);
    for (let index = 1; index < chain.length; index += 1) {
      vertices.get(chain[index - 1])!.succs.push(chain[index]);
      vertices.get(chain[index])!.preds.push(chain[index - 1]);
    }
    chains.set(key, chain);
  }

  // 与谁都不相连的节点（至多带自环）先不排，免得把有关系的部分挤开；排好后再补进较短的列。
  const isolated = nodes
    .map((node) => node.id)
    .filter((id) => {
      const vertex = vertices.get(id)!;
      return !vertex.preds.length && !vertex.succs.length;
    });
  const loose = new Set(isolated);
  const connected = [...vertices.values()].filter(
    (vertex) => !loose.has(vertex.id),
  );
  const columns: string[][] = Array.from(
    {
      length: connected.length
        ? Math.max(...connected.map((vertex) => vertex.layer)) + 1
        : 0,
    },
    () => [],
  );
  for (const vertex of connected) columns[vertex.layer].push(vertex.id);
  const position = new Map<string, number>();
  for (const column of columns) {
    column.sort(
      (left, right) =>
        vertices.get(left)!.seed - vertices.get(right)!.seed ||
        (left < right ? -1 : left > right ? 1 : 0),
    );
    column.forEach((id, index) => position.set(id, index));
  }
  const reorder = (
    column: string[],
    neighbours: (vertex: Vertex) => string[],
  ) => {
    const weight = new Map(
      column.map((id) => {
        const around = neighbours(vertices.get(id)!);
        return [
          id,
          around.length
            ? around.reduce((sum, other) => sum + position.get(other)!, 0) /
              around.length
            : position.get(id)!,
        ] as const;
      }),
    );
    column.sort((left, right) => weight.get(left)! - weight.get(right)!);
    column.forEach((id, index) => position.set(id, index));
  };
  for (let pass = 0; pass < 4; pass += 1) {
    for (let index = 1; index < columns.length; index += 1)
      reorder(columns[index], (vertex) => vertex.preds);
    for (let index = columns.length - 2; index >= 0; index -= 1)
      reorder(columns[index], (vertex) => vertex.succs);
  }
  // 不相连的节点按声明顺序补进不满的列，满了向右加列；每列至多约 √节点数 个（至少 4 个）
  const limit = Math.max(
    4,
    Math.ceil(Math.sqrt(nodes.length)),
    ...columns.map((column) => column.length),
  );
  for (const id of isolated) {
    let target = columns.findIndex((column) => column.length < limit);
    if (target < 0) target = columns.push([]) - 1;
    columns[target].push(id);
    vertices.get(id)!.layer = target;
  }
  if (!columns.length) columns.push([]);
  const layerCount = columns.length;

  const row = new Map<string, number>();
  for (const column of columns) {
    let previous = -1;
    for (const id of column) {
      const preds = vertices.get(id)!.preds;
      const wanted = preds.length
        ? lowerMedian(preds.map((other) => row.get(other)!))
        : previous + 1;
      previous = Math.max(wanted, previous + 1);
      row.set(id, previous);
    }
  }
  for (let index = layerCount - 1; index >= 0; index -= 1) {
    const column = columns[index];
    for (let slot = column.length - 1; slot >= 0; slot -= 1) {
      const vertex = vertices.get(column[slot])!;
      if (vertex.preds.length || !vertex.succs.length) continue;
      const low = slot > 0 ? row.get(column[slot - 1])! + 1 : 0;
      const high =
        slot + 1 < column.length
          ? row.get(column[slot + 1])! - 1
          : Number.POSITIVE_INFINITY;
      const wanted = lowerMedian(vertex.succs.map((other) => row.get(other)!));
      if (low <= high)
        row.set(vertex.id, Math.min(Math.max(wanted, low), high));
    }
  }
  const used = [...new Set(row.values())].sort((left, right) => left - right);
  const compact = new Map(used.map((value, index) => [value, index]));
  for (const [id, value] of row) row.set(id, compact.get(value)!);
  const rowCount = Math.max(1, used.length);
  const top = (id: string) => PADDING + row.get(id)! * (BOX_HEIGHT + ROW_GAP);
  const middle = (id: string) => top(id) + BOX_HEIGHT / 2;
  const layerOf = (id: string) => vertices.get(id)!.layer;

  // 每条关系经过的站点：起点、占位点……终点；逆着分层方向的关系把链倒过来走。
  type Route = { wire: number; stations: string[]; forward: boolean };
  const routes: Route[] = [];
  const loops = new Map<string, number[]>();
  relations.forEach((relation, wire) => {
    if (relation.from === relation.to) {
      loops.set(relation.from, [...(loops.get(relation.from) ?? []), wire]);
      return;
    }
    const [head, tail] = oriented(relation.from, relation.to);
    const chain = chains.get(edgeKey(head, tail))!;
    const forward = head === relation.from;
    routes.push({
      wire,
      stations: forward ? chain : [...chain].reverse(),
      forward,
    });
  });

  // 接点：对端与本框同一行的接在中线上（直线不拐弯），对端在上方的依次排在中线以上，
  // 在下方的排在以下，越远的越靠外（出框后不交叉）；占位点里的多条线上下错开。
  const ports = new Map<string, { wire: number; toward: number }[]>();
  const lanes = new Map<string, number[]>();
  const portKey = (id: string, side: 'left' | 'right') => `${id}|${side}`;
  for (const { wire, stations, forward } of routes) {
    const first = stations[0];
    const last = stations[stations.length - 1];
    const out = portKey(first, forward ? 'right' : 'left');
    const into = portKey(last, forward ? 'left' : 'right');
    ports.set(out, [
      ...(ports.get(out) ?? []),
      { wire, toward: middle(stations[1]) },
    ]);
    ports.set(into, [
      ...(ports.get(into) ?? []),
      { wire, toward: middle(stations[stations.length - 2]) },
    ]);
    for (const id of stations.slice(1, -1))
      lanes.set(id, [...(lanes.get(id) ?? []), wire]);
  }
  const spread = (count: number, index: number, step: number) =>
    (index - (count - 1) / 2) * step;
  const portY = new Map<string, number>();
  for (const [key, list] of ports) {
    const center = middle(key.slice(0, key.lastIndexOf('|')));
    const sorted = [...list].sort(
      (left, right) => left.toward - right.toward || left.wire - right.wire,
    );
    const level = (toward: number) =>
      Math.abs(toward - center) < 1 ? 0 : Math.sign(toward - center);
    const same = sorted.filter(({ toward }) => level(toward) === 0);
    if (!same.length) {
      // 没有同一行的对端：按对端高低对称排开，只有一条时正好在中线上
      const step = Math.min(
        PORT_STEP,
        (2 * PORT_REACH) / Math.max(1, sorted.length - 1),
      );
      sorted.forEach(({ wire }, index) =>
        portY.set(
          `${key}|${wire}`,
          center + spread(sorted.length, index, step),
        ),
      );
      continue;
    }
    const above = sorted.filter(({ toward }) => level(toward) < 0);
    const below = sorted.filter(({ toward }) => level(toward) > 0);
    const half = (same.length - 1) / 2;
    const step = Math.min(
      PORT_STEP,
      PORT_REACH / Math.max(1, Math.max(above.length, below.length) + half),
    );
    same.forEach(({ wire }, index) =>
      portY.set(`${key}|${wire}`, center + spread(same.length, index, step)),
    );
    above.forEach(({ wire }, index) =>
      portY.set(
        `${key}|${wire}`,
        center - (half + above.length - index) * step,
      ),
    );
    below.forEach(({ wire }, index) =>
      portY.set(`${key}|${wire}`, center + (half + 1 + index) * step),
    );
  }
  const laneY = new Map<string, number>();
  for (const [id, list] of lanes)
    list.forEach((wire, index) =>
      laneY.set(
        `${id}|${wire}`,
        middle(id) + spread(list.length, index, PORT_STEP),
      ),
    );
  const heights = routes.map(({ wire, stations, forward }) =>
    stations.map((id, index) =>
      index === 0
        ? portY.get(`${portKey(id, forward ? 'right' : 'left')}|${wire}`)!
        : index === stations.length - 1
          ? portY.get(`${portKey(id, forward ? 'left' : 'right')}|${wire}`)!
          : laneY.get(`${id}|${wire}`)!,
    ),
  );

  // 通道里的竖线：记下每段在左列、右列的高度。从左往右看往上走的，左端越高越靠左；
  // 往下走的，左端越低越靠左——两条同向的线这样排不交叉。
  const tracks = new Map<string, number>();
  const channels: { key: string; leftY: number; rightY: number }[][] =
    Array.from({ length: Math.max(0, layerCount - 1) }, () => []);
  routes.forEach(({ wire, stations }, route) => {
    for (let index = 0; index + 1 < stations.length; index += 1) {
      const [from, to] = [heights[route][index], heights[route][index + 1]];
      if (Math.abs(from - to) < 1) continue;
      const leftFirst = layerOf(stations[index]) < layerOf(stations[index + 1]);
      const channel = Math.min(
        layerOf(stations[index]),
        layerOf(stations[index + 1]),
      );
      channels[channel].push({
        key: `${wire}|${index}`,
        leftY: leftFirst ? from : to,
        rightY: leftFirst ? to : from,
      });
    }
  });
  const widths = channels.map((hops) => {
    const upward = hops
      .filter((hop) => hop.rightY < hop.leftY)
      .sort((left, right) => left.leftY - right.leftY);
    const downward = hops
      .filter((hop) => hop.rightY > hop.leftY)
      .sort((left, right) => right.leftY - left.leftY);
    [...upward, ...downward].forEach((hop, index) =>
      tracks.set(hop.key, index),
    );
    return Math.max(
      CHANNEL_MIN,
      2 * CHANNEL_MARGIN + (hops.length + 1) * TRACK_STEP,
    );
  });
  const lefts = [PADDING];
  for (let index = 1; index < layerCount; index += 1)
    lefts.push(lefts[index - 1] + BOX_WIDTH + widths[index - 1]);
  const leftOf = (id: string) => lefts[layerOf(id)];
  const trackX = (channel: number, track: number) =>
    lefts[channel] +
    BOX_WIDTH +
    CHANNEL_MARGIN +
    ((track + 1) * (widths[channel] - 2 * CHANNEL_MARGIN)) /
      (channels[channel].length + 1);

  const wires: Wire[] = [];
  routes.forEach(({ wire, stations, forward }, route) => {
    const direction = forward ? 1 : -1;
    const exitX = (id: string) => leftOf(id) + (forward ? BOX_WIDTH : 0);
    const entryX = (id: string) => leftOf(id) + (forward ? 0 : BOX_WIDTH);
    const points: [number, number][] = [
      [exitX(stations[0]), heights[route][0]],
    ];
    for (let index = 0; index + 1 < stations.length; index += 1) {
      const target = stations[index + 1];
      const [from, to] = [heights[route][index], heights[route][index + 1]];
      const track = tracks.get(`${wire}|${index}`);
      if (track !== undefined) {
        const channel = Math.min(layerOf(stations[index]), layerOf(target));
        const x = trackX(channel, track);
        points.push([x, from], [x, to]);
      }
      points.push([entryX(target), to]);
      if (index + 1 < stations.length - 1) points.push([exitX(target), to]);
    }
    const [tipX, tipY] = points[points.length - 1];
    points[points.length - 1] = [tipX - direction * HEAD, tipY];
    wires.push({
      index: wire,
      relation: relations[wire],
      line: pathOf(points),
      head: pathOf([
        [tipX, tipY],
        [tipX - direction * HEAD, tipY - HEAD / 2],
        [tipX - direction * HEAD, tipY + HEAD / 2],
      ]).concat(' Z'),
    });
  });
  for (const [id, list] of loops)
    list.forEach((wire, index) => {
      const y = top(id);
      const end = leftOf(id) + BOX_WIDTH - 14 - index * (LOOP_WIDTH + 10);
      const start = end - LOOP_WIDTH;
      wires.push({
        index: wire,
        relation: relations[wire],
        line: pathOf([
          [start, y],
          [start, y - LOOP_HEIGHT],
          [end, y - LOOP_HEIGHT],
          [end, y - HEAD],
        ]),
        head: pathOf([
          [end, y],
          [end - HEAD / 2, y - HEAD],
          [end + HEAD / 2, y - HEAD],
        ]).concat(' Z'),
      });
    });
  wires.sort((left, right) => left.index - right.index);

  return {
    width: lefts[layerCount - 1] + BOX_WIDTH + PADDING,
    height: 2 * PADDING + rowCount * BOX_HEIGHT + (rowCount - 1) * ROW_GAP,
    boxes: new Map(
      nodes.map((node) => [node.id, { x: leftOf(node.id), y: top(node.id) }]),
    ),
    wires,
  };
}

/**
 * 模块关系：整行的关系图（网格纸底；节点黑框，依赖实线、数据流虚线，1.3px，箭头指向终点），
 * 下接关系明细（8 列：筛选、起点 → 终点、类型与说明、原文）与所选节点（4 列）。
 */
export function ArchitectureRelations({
  architecture,
  data,
  onOpen,
}: {
  architecture: ArchitectureData;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [selection, setSelection] = useState('');
  const [kind, setKind] = useState<'all' | ArchitectureRelation['kind']>('all');
  const nodes = new Map(architecture.nodes.map((node) => [node.id, node]));
  const selected = nodes.get(selection) ?? architecture.nodes[0];
  const drawing = useMemo(
    () => layoutDrawing(architecture.nodes, architecture.relations),
    [architecture.nodes, architecture.relations],
  );
  const relations = architecture.relations.filter(
    (relation) => kind === 'all' || relation.kind === kind,
  );
  const wires = drawing.wires.filter(
    (wire) => kind === 'all' || wire.relation.kind === kind,
  );
  return (
    <>
      <Cell span={12} surface="grid" as="div">
        <div className="architecture-drawing">
          <div
            className="architecture-canvas"
            role="group"
            aria-label="架构节点"
            style={{
              width: drawing.width,
              height: drawing.height,
              ...cssVars({
                '--architecture-node-width': `${BOX_WIDTH}px`,
                '--architecture-node-height': `${BOX_HEIGHT}px`,
              }),
            }}
          >
            <svg
              className="architecture-wires"
              width={drawing.width}
              height={drawing.height}
              viewBox={`0 0 ${drawing.width} ${drawing.height}`}
              aria-hidden="true"
              focusable="false"
            >
              {wires.map((wire) => (
                <g
                  key={wire.index}
                  className={cx(
                    'architecture-wire',
                    wire.relation.kind === 'data_flow'
                      ? 'is-data-flow'
                      : 'is-dependency',
                    (wire.relation.from === selected.id ||
                      wire.relation.to === selected.id) &&
                      'is-active',
                  )}
                >
                  <path className="architecture-wire__line" d={wire.line} />
                  <path className="architecture-wire__head" d={wire.head} />
                </g>
              ))}
            </svg>
            {architecture.nodes.map((node) => {
              const box = drawing.boxes.get(node.id)!;
              return (
                <button
                  key={node.id}
                  type="button"
                  className="architecture-node"
                  style={{ left: box.x, top: box.y }}
                  aria-pressed={node.id === selected.id}
                  onClick={() => setSelection(node.id)}
                  aria-label={`选择架构节点 ${node.label}`}
                  title={node.label}
                >
                  <span className="architecture-node__kind">
                    {nodeKinds[node.kind]}
                  </span>
                  <span className="architecture-node__label">{node.label}</span>
                </button>
              );
            })}
          </div>
        </div>
        <p className="architecture-legend">
          <span>
            <span className="architecture-legend__line" aria-hidden="true" />
            实线 依赖声明
          </span>
          <span>
            <span
              className="architecture-legend__line is-data-flow"
              aria-hidden="true"
            />
            虚线 数据流声明
          </span>
          <span>
            <span className="architecture-legend__box" aria-hidden="true" />
            实心 所选节点，相连的线加粗
          </span>
        </p>
      </Cell>
      <Cell span={8} as="div">
        <div className="architecture-filter">
          <label htmlFor="architecture-relation-kind">关系类型</label>
          <span className="project-field project-field--select">
            <select
              id="architecture-relation-kind"
              value={kind}
              onChange={(event) => setKind(event.target.value as typeof kind)}
            >
              <option value="all">全部关系</option>
              <option value="dependency">依赖声明</option>
              <option value="data_flow">数据流声明</option>
            </select>
            <Icon name="chevron" />
          </span>
          <span className="operation-muted">{relations.length} 条文档声明</span>
        </div>
        {relations.length ? (
          <ul className="architecture-edge-list" aria-label="架构关系声明">
            {relations.map((relation, index) => (
              <li
                key={`${relation.from}:${relation.to}:${relation.kind}:${index}`}
              >
                <div className="architecture-edge-endpoints">
                  <button
                    type="button"
                    className="architecture-endpoint"
                    onClick={() => setSelection(relation.from)}
                    aria-label={`选择关系起点 ${nodes.get(relation.from)!.label}`}
                  >
                    {nodes.get(relation.from)!.label}
                  </button>
                  <span
                    className={cx(
                      'architecture-edge-arrow',
                      relation.kind === 'data_flow' && 'is-data-flow',
                    )}
                    aria-hidden="true"
                  />
                  <button
                    type="button"
                    className="architecture-endpoint"
                    onClick={() => setSelection(relation.to)}
                    aria-label={`选择关系终点 ${nodes.get(relation.to)!.label}`}
                  >
                    {nodes.get(relation.to)!.label}
                  </button>
                </div>
                <p>
                  <span className="architecture-relation-kind">
                    {relationKinds[relation.kind]}
                  </span>
                  {relation.label || '未提供关系说明'}
                </p>
                <Button
                  variant="text"
                  size="sm"
                  onClick={() => onOpen(relation.source)}
                  aria-label={`${nodes.get(relation.from)!.label}到${nodes.get(relation.to)!.label}，查看${relationKinds[relation.kind]}来源`}
                >
                  查看关系原文
                </Button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="operation-muted" role="status">
            当前类型没有关系声明。
          </p>
        )}
      </Cell>
      <ArchitectureNodeDetails node={selected} data={data} onOpen={onOpen} />
    </>
  );
}
