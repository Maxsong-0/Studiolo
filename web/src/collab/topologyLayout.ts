/*
 * 协作拓扑的布局（纯函数，不碰 React 与 DOM）。设计规范第 7 节：
 * 三列——执行者 x=0（宽 170）、任务 x=300（宽 240）、占用路径 x=690（宽 300），坐标系 1030 宽；
 * 按执行者分组、组内按任务 id 排序；路径对齐到任务行，多条时上下对称展开；
 * 连线全是正交折线：执行者右缘 → x=235 → 任务左缘，任务右缘 → x=600 → 路径左缘。
 * 写域重叠与 control-db 的 `contains` 同一规则；休眠、租约到期都按传入的 now 判，不读本机时钟。
 */
import type { BoardOverviewItem } from '../boardOverviewModel';
import type { StatusState } from '../ui';
import {
  isClaimedWork,
  isFinished,
  isWaiting,
  statusLabels,
  statusMark,
} from './boardLabels';
import {
  executorLabels,
  executorOf,
  groupByExecutor,
  type ExecutorGroup,
} from './executorGrouping';

export const TOPOLOGY_WIDTH = 1030;
/** 认领后超过这么久没有进度回报，就算休眠。 */
export const DORMANT_AFTER_MS = 6 * 3_600_000;
export const DEFAULT_MINI_TASKS = 6;

export const COLUMNS = {
  executor: { x: 0, width: 170 },
  task: { x: 300, width: 240 },
  path: { x: 690, width: 300 },
} as const;
export const EXECUTOR_BUS_X = 235;
export const PATH_BUS_X = 600;

const EXECUTOR_HEIGHT = 44;
const TASK_HEIGHT = 40;
const PATH_HEIGHT = 30;
/** 重叠路径框下面那一行「写域重叠：…」占的高度。 */
const NOTE_HEIGHT = 18;
const PATH_GAP = 10;
const ROW_GAP = 20;
const GROUP_GAP = 10;
/** 全休眠的执行者排在最后，前面多留一段空。 */
const DORMANT_GAP = 20;
/** 首行上沿：上面留给三列的列名。 */
const TOP = 20;
const BOTTOM = 12;
const INSET = 12;
const MARKER_RADIUS = 11;
const BRACKET_X = COLUMNS.path.x + COLUMNS.path.width + 2;
const BRACKET_DEPTH = 14;
const BRACKET_STEP = 5;
const BRACKET_LIMIT = TOPOLOGY_WIDTH - 2;

const EXECUTOR_RIGHT = COLUMNS.executor.x + COLUMNS.executor.width;
const TASK_RIGHT = COLUMNS.task.x + COLUMNS.task.width;
const PATH_RIGHT = COLUMNS.path.x + COLUMNS.path.width;

/** 圈号标注的目标：一条任务，或一条占用路径（与 TopologyDiagram 的 TopologyMarker 同形）。 */
export interface LayoutMarker {
  n: number;
  target: { taskId: string } | { path: string };
}

/** 一处写域重叠：两条不同任务上的两条路径，a、b 按（任务 id, 路径）排好。 */
export interface PathOverlap {
  a: { taskId: string; path: string };
  b: { taskId: string; path: string };
}

export type OverlapRelation = 'equal' | 'contains' | 'within';

export interface TopologyLayoutOptions {
  mode: 'mini' | 'full';
  /** 判休眠与租约到期用的「此刻」（毫秒）。 */
  now: number;
  /** mini 模式最多画几条任务，缺省 6。 */
  maxTasks?: number;
  /** 画不画休眠任务，缺省画。 */
  showDormant?: boolean;
  /** 只留写域重叠相关的任务与路径。 */
  onlyConflicts?: boolean;
  markers?: readonly LayoutMarker[];
  selectedTaskId?: string | null;
}

export type TaskTone = 'normal' | 'muted' | 'waiting' | 'alert' | 'dormant';

export interface ExecutorRow {
  key: string;
  label: string;
  /** 画在框里的标签（按框宽截断）。 */
  name: string;
  /** 第二行：「N 条任务」「休眠 · N 条」「未认领 · N 条」。 */
  detail: string;
  /** 本组画出来的任务全是休眠。 */
  dormant: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  centerY: number;
}

export interface TaskRow {
  id: string;
  item: BoardOverviewItem;
  executorKey: string;
  /** 状态说法（休眠、租约到期优先），不带方块字符。 */
  status: string;
  state: StatusState;
  tone: TaskTone;
  /** 框里右上角的状态文字，含 ■/□。 */
  statusText: string;
  /** 框里左上角：项目名，选中时加「已选中」。 */
  caption: string;
  /** 框里第二行的标题（按框宽截断）。 */
  titleText: string;
  dormant: boolean;
  leaseExpired: boolean;
  selected: boolean;
  overlapping: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  centerY: number;
}

export interface PathRow {
  key: string;
  taskId: string;
  path: string;
  /** 框里的路径文字（太长时截掉开头，留住最具体的尾巴）。 */
  text: string;
  overlaps: PathOverlap[];
  /** 完整的「写域重叠：…」；没有重叠为 null。 */
  note: string | null;
  /** 画在框下的那一行（按宽截断）。 */
  noteText: string | null;
  noteY: number | null;
  dormant: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  centerY: number;
}

export interface TopologyEdge {
  key: string;
  kind: 'claim' | 'path';
  d: string;
  dormant: boolean;
  /** 选中任务的连线加粗。 */
  emphasis: boolean;
}

export interface OverlapBracket {
  key: string;
  d: string;
}

export interface MarkerPlacement {
  key: string;
  n: number;
  cx: number;
  cy: number;
  r: number;
  leader: { x1: number; y1: number; x2: number; y2: number };
  /** 圈号的完整说法，给 <title> 与清单。 */
  label: string;
  target: { taskId: string } | { pathKey: string };
}

export interface TopologyLayout {
  width: number;
  height: number;
  executors: ExecutorRow[];
  tasks: TaskRow[];
  paths: PathRow[];
  edges: TopologyEdge[];
  brackets: OverlapBracket[];
  markers: MarkerPlacement[];
  /** 未结束条目（含休眠）之间的全部写域重叠，不受显示开关与截断影响。 */
  overlaps: PathOverlap[];
  /** 未结束条目里判为休眠的任务 id，升序。 */
  dormant: string[];
  /** mini 模式按 maxTasks 截掉的条数；full 恒为 0。 */
  omitted: number;
  /** 被显示开关藏掉的条数（未开「显示休眠」时的休眠任务、开了「只看冲突」时的无关任务）。 */
  hidden: number;
  /** 未结束条目总数。 */
  unfinished: number;
}

/* ---------- 规则 ---------- */

/**
 * 与 control-db `contains` 一致：两条相等，或在 `/` 边界上 root 是 path 的前缀；比较前转小写。
 * 例：`web` 包含 `web/src/policyApi.ts`，不包含 `webapp`。
 */
export function pathContains(root: string, path: string): boolean {
  const left = root.toLowerCase();
  const right = path.toLowerCase();
  return (
    left === right ||
    (right.startsWith(left) && right.charAt(left.length) === '/')
  );
}

export function overlapRelation(path: string, other: string): OverlapRelation {
  if (path.toLowerCase() === other.toLowerCase()) return 'equal';
  return pathContains(path, other) ? 'contains' : 'within';
}

const byText = (left: string, right: string) =>
  left < right ? -1 : left > right ? 1 : 0;
const sideOrder = (
  left: { taskId: string; path: string },
  right: { taskId: string; path: string },
) => byText(left.taskId, right.taskId) || byText(left.path, right.path);
const pathKey = (taskId: string, path: string) => `${taskId}\u0000${path}`;

/**
 * 未结束条目（含休眠）之间的写域重叠；同一任务自己的路径互相嵌套不算。
 * 每条路径只查自己与各级 `/` 祖先，条目多、路径多时也不做两两全比。
 */
export function findOverlaps(
  items: readonly BoardOverviewItem[],
): PathOverlap[] {
  type Entry = { taskId: string; path: string; key: string };
  const entries: Entry[] = [];
  const byKey = new Map<string, Entry[]>();
  for (const item of items) {
    if (isFinished(item.status)) continue;
    for (const path of item.paths) {
      const entry = { taskId: item.id, path, key: path.toLowerCase() };
      entries.push(entry);
      const same = byKey.get(entry.key);
      if (same) same.push(entry);
      else byKey.set(entry.key, [entry]);
    }
  }
  const seen = new Set<string>();
  const overlaps: PathOverlap[] = [];
  for (const entry of entries) {
    for (
      let end = entry.key.length;
      end > 0;
      end = entry.key.lastIndexOf('/', end - 1)
    ) {
      for (const other of byKey.get(entry.key.slice(0, end)) ?? []) {
        if (other.taskId === entry.taskId) continue;
        const [a, b] = [other, entry].sort(sideOrder);
        const id = `${pathKey(a.taskId, a.path)}\u0000${pathKey(b.taskId, b.path)}`;
        if (seen.has(id)) continue;
        seen.add(id);
        overlaps.push({
          a: { taskId: a.taskId, path: a.path },
          b: { taskId: b.taskId, path: b.path },
        });
      }
    }
  }
  return overlaps.sort(
    (left, right) => sideOrder(left.a, right.a) || sideOrder(left.b, right.b),
  );
}

/**
 * 休眠：没人在推进、也不在等主人（`needs_*` 与已结束的都不算）。两种情形：
 * 从未认领、也没有任何进度（草稿、待认领）；或处于认领三态（claimed/running/committing），
 * 却没有活跃尝试，或最近一次进度回报距 now 超过 6 小时。认领三态但从未回报进度的，无从判断，不算休眠。
 */
export function isDormant(item: BoardOverviewItem, now: number): boolean {
  if (isFinished(item.status) || isWaiting(item.status)) return false;
  if (item.attempt === null && item.progress === null) return true;
  if (!isClaimedWork(item.status)) return false;
  if (item.attempt === null || !item.attempt.active) return true;
  return (
    item.progress !== null &&
    now - item.progress.lastProgressMs > DORMANT_AFTER_MS
  );
}

/** 与黑板视图 boardLease 的「expired」同一口径：活跃的认领或执行，租约截止已过。 */
export function isLeaseExpired(item: BoardOverviewItem, now: number): boolean {
  const attempt = item.attempt;
  return (
    attempt !== null &&
    attempt.active &&
    (attempt.phase === 'claimed' || attempt.phase === 'running') &&
    attempt.leaseUntilMs <= now
  );
}

export interface TopologySummary {
  /** 未结束且未休眠。 */
  running: number;
  /** 写域重叠的处数（路径对）。 */
  overlaps: number;
  /** 待审阅、等待修订、等待重新确认。 */
  waiting: number;
  dormant: number;
}

/** 协作页右上角的汇总，只数未结束的条目。 */
export function topologySummary(
  items: readonly BoardOverviewItem[],
  now: number,
): TopologySummary {
  const unfinished = items.filter((item) => !isFinished(item.status));
  const dormant = unfinished.filter((item) => isDormant(item, now)).length;
  return {
    running: unfinished.length - dormant,
    overlaps: findOverlaps(unfinished).length,
    waiting: unfinished.filter((item) => isWaiting(item.status)).length,
    dormant,
  };
}

/** 任务在图上的状态说法、方块与色调：休眠优先，其次租约到期，再按任务状态。 */
export function taskStatus(
  item: BoardOverviewItem,
  now: number,
): { status: string; state: StatusState; tone: TaskTone } {
  if (isDormant(item, now))
    return { status: '休眠', state: 'dormant', tone: 'dormant' };
  if (isLeaseExpired(item, now))
    return { status: '租约已到期', state: 'alert', tone: 'alert' };
  const state = statusMark(item.status);
  return {
    status: statusLabels[item.status],
    state,
    tone: isWaiting(item.status)
      ? 'waiting'
      : state === 'alert'
        ? 'alert'
        : state === 'ok' || state === 'dormant'
          ? 'muted'
          : 'normal',
  };
}

/** 项目路径的最后一段（`项目/Studiolo` → `Studiolo`）。 */
export function projectName(project: string): string {
  return project.slice(project.lastIndexOf('/') + 1) || project;
}

/* ---------- 文字宽度（估算） ---------- */

function isWide(code: number): boolean {
  return (
    (code >= 0x1100 && code <= 0x115f) ||
    (code >= 0x2e80 && code <= 0xa4cf && code !== 0x303f) ||
    (code >= 0xac00 && code <= 0xd7a3) ||
    (code >= 0xf900 && code <= 0xfaff) ||
    (code >= 0xfe30 && code <= 0xfe4f) ||
    (code >= 0xff00 && code <= 0xff60) ||
    (code >= 0xffe0 && code <= 0xffe6) ||
    (code >= 0x20000 && code <= 0x3fffd)
  );
}

/** 估算一行字的宽度：全角按 1em，空格 0.3em，其余 0.6em（Archivo 粗体与 Plex Mono 的平均字宽）。 */
export function textWidth(text: string, size: number): number {
  let width = 0;
  for (const character of text) {
    const code = character.codePointAt(0) ?? 0;
    width += size * (isWide(code) ? 1 : character === ' ' ? 0.3 : 0.6);
  }
  return width;
}

/** 放不下就截断并补「…」；keep=end 时截掉开头、留住结尾（路径的尾巴最具体）。 */
export function fitText(
  text: string,
  maxWidth: number,
  size: number,
  keep: 'start' | 'end' = 'start',
): string {
  if (textWidth(text, size) <= maxWidth) return text;
  const room = maxWidth - textWidth('…', size);
  const characters = Array.from(text);
  const kept: string[] = [];
  let width = 0;
  for (const character of keep === 'end'
    ? [...characters].reverse()
    : characters) {
    const next = textWidth(character, size);
    if (width + next > room) break;
    kept.push(character);
    width += next;
  }
  return keep === 'end' ? `…${kept.reverse().join('')}` : `${kept.join('')}…`;
}

/* ---------- 布局 ---------- */

const EXECUTOR_TEXT = COLUMNS.executor.width - INSET * 2;
const TASK_TEXT = COLUMNS.task.width - INSET * 2;
const PATH_TEXT = COLUMNS.path.width - INSET * 2;
const NOTE_TEXT = COLUMNS.path.width;

/** mini 模式挑任务的先后：圈号指到的 > 等你/租约到期 > 写域重叠 > 在跑 > 其余 > 休眠。 */
function tier(
  item: BoardOverviewItem,
  now: number,
  marked: ReadonlySet<string>,
  conflicted: ReadonlySet<string>,
  dormant: ReadonlySet<string>,
): number {
  if (marked.has(item.id)) return 0;
  if (dormant.has(item.id)) return 5;
  if (isWaiting(item.status) || isLeaseExpired(item, now)) return 1;
  if (conflicted.has(item.id)) return 2;
  if (item.attempt?.active) return 3;
  return 4;
}

/** 组序沿用 groupByExecutor，全休眠的组挪到最后（相对次序不变）。 */
function orderGroups(
  items: readonly BoardOverviewItem[],
  labels: ReadonlyMap<string, string>,
  dormant: ReadonlySet<string>,
): { group: ExecutorGroup; dormant: boolean }[] {
  const groups = groupByExecutor(items, labels).map((group) => ({
    group,
    dormant: group.items.every((item) => dormant.has(item.id)),
  }));
  return [
    ...groups.filter((entry) => !entry.dormant),
    ...groups.filter((entry) => entry.dormant),
  ];
}

function overlapNote(
  taskId: string,
  path: string,
  overlaps: readonly PathOverlap[],
  byId: ReadonlyMap<string, BoardOverviewItem>,
  now: number,
): { note: string; short: string } {
  const first = overlaps[0];
  const other =
    first.a.taskId === taskId && first.a.path === path ? first.b : first.a;
  const relation = overlapRelation(path, other.path);
  const head =
    relation === 'equal'
      ? `同为 ${other.path}`
      : relation === 'contains'
        ? `包含 ${other.path}`
        : `包含于 ${other.path}`;
  const owner = byId.get(other.taskId);
  const who = owner ? owner.title : other.taskId;
  const status = owner ? taskStatus(owner, now).status : null;
  const more = overlaps.length > 1 ? ` 等 ${overlaps.length} 处` : '';
  const full = `写域重叠：${head}（${who}${status ? ` · ${status}` : ''}）${more}`;
  const plain = `写域重叠：${head}（${who}）${more}`;
  const short =
    textWidth(full, 11) <= NOTE_TEXT
      ? full
      : fitText(plain, NOTE_TEXT, 11, 'start');
  return { note: full, short };
}

/**
 * 计算整张拓扑：只画未结束的任务；休眠受 showDormant、无关条目受 onlyConflicts 控制；
 * mini 模式最多 maxTasks 条（按 tier 挑，挑中的仍按组序排）并报告截掉的条数。
 */
export function topologyLayout(
  items: readonly BoardOverviewItem[],
  options: TopologyLayoutOptions,
): TopologyLayout {
  const { mode, now } = options;
  const showDormant = options.showDormant ?? true;
  const onlyConflicts = options.onlyConflicts ?? false;
  const markers = options.markers ?? [];
  const selectedTaskId = options.selectedTaskId ?? null;
  const limit =
    mode === 'mini'
      ? Number.isFinite(options.maxTasks) && (options.maxTasks ?? 0) >= 1
        ? Math.floor(options.maxTasks ?? DEFAULT_MINI_TASKS)
        : DEFAULT_MINI_TASKS
      : Number.POSITIVE_INFINITY;

  const unfinished = items.filter((item) => !isFinished(item.status));
  const byId = new Map(unfinished.map((item) => [item.id, item]));
  const overlaps = findOverlaps(unfinished);
  const dormant = new Set(
    unfinished.filter((item) => isDormant(item, now)).map((item) => item.id),
  );
  const overlapsByPath = new Map<string, PathOverlap[]>();
  for (const overlap of overlaps)
    for (const side of [overlap.a, overlap.b]) {
      const key = pathKey(side.taskId, side.path);
      const list = overlapsByPath.get(key);
      if (list) list.push(overlap);
      else overlapsByPath.set(key, [overlap]);
    }
  const conflicted = new Set(
    overlaps.flatMap((overlap) => [overlap.a.taskId, overlap.b.taskId]),
  );

  const shown = unfinished.filter(
    (item) =>
      (showDormant || !dormant.has(item.id)) &&
      (!onlyConflicts || conflicted.has(item.id)),
  );
  const hidden = unfinished.length - shown.length;
  const labels = executorLabels(items.map(executorOf));

  let groups = orderGroups(shown, labels, dormant);
  let omitted = 0;
  if (shown.length > limit) {
    const marked = new Set<string>();
    for (const marker of markers) {
      const target = marker.target;
      if ('taskId' in target) marked.add(target.taskId);
      else
        for (const item of shown)
          if (item.paths.some((path) => samePath(path, target.path)))
            marked.add(item.id);
    }
    const ranked = groups
      .flatMap((entry) => entry.group.items)
      .map((item, index) => ({
        item,
        index,
        tier: tier(item, now, marked, conflicted, dormant),
      }))
      .sort((left, right) => left.tier - right.tier || left.index - right.index)
      .slice(0, limit);
    const picked = new Set(ranked.map((entry) => entry.item.id));
    omitted = shown.length - picked.size;
    groups = orderGroups(
      shown.filter((item) => picked.has(item.id)),
      labels,
      dormant,
    );
  }

  const executors: ExecutorRow[] = [];
  const tasks: TaskRow[] = [];
  const paths: PathRow[] = [];
  const edges: TopologyEdge[] = [];
  let y = TOP;
  groups.forEach(({ group, dormant: sleeping }, index) => {
    if (index > 0) y += GROUP_GAP;
    if (sleeping && index > 0 && !groups[index - 1].dormant) y += DORMANT_GAP;
    const centers: number[] = [];
    for (const item of group.items) {
      const own = onlyConflicts
        ? item.paths.filter((path) =>
            overlapsByPath.has(pathKey(item.id, path)),
          )
        : item.paths;
      const heights = own.map(
        (path) =>
          PATH_HEIGHT +
          (overlapsByPath.has(pathKey(item.id, path)) ? NOTE_HEIGHT : 0),
      );
      const stack =
        heights.reduce((sum, height) => sum + height, 0) +
        Math.max(0, heights.length - 1) * PATH_GAP;
      const content = Math.max(TASK_HEIGHT, stack);
      const centerY = y + content / 2;
      const taskY = centerY - TASK_HEIGHT / 2;
      const selected = item.id === selectedTaskId;
      const isSleeping = dormant.has(item.id);
      const { status, state, tone } = taskStatus(item, now);
      const mark = state === 'active' ? '■ ' : state === 'pending' ? '□ ' : '';
      const statusText = fitText(`${mark}${status}`, TASK_TEXT / 2, 10.5);
      const captionRoom = TASK_TEXT - textWidth(statusText, 10.5) - 10;
      tasks.push({
        id: item.id,
        item,
        executorKey: group.key,
        status,
        state,
        tone,
        statusText,
        caption: fitText(
          `${projectName(item.project)}${selected ? ' · 已选中' : ''}`,
          captionRoom,
          10,
        ),
        titleText: fitText(item.title, TASK_TEXT, 13.5),
        dormant: isSleeping,
        leaseExpired: isLeaseExpired(item, now),
        selected,
        overlapping: conflicted.has(item.id),
        x: COLUMNS.task.x,
        y: taskY,
        width: COLUMNS.task.width,
        height: TASK_HEIGHT,
        centerY,
      });
      let top = centerY - stack / 2;
      own.forEach((path, pathIndex) => {
        const key = pathKey(item.id, path);
        const list = overlapsByPath.get(key) ?? [];
        const note = list.length
          ? overlapNote(item.id, path, list, byId, now)
          : null;
        const boxCenter = top + PATH_HEIGHT / 2;
        paths.push({
          key,
          taskId: item.id,
          path,
          text: fitText(path, PATH_TEXT, 12, 'end'),
          overlaps: list,
          note: note?.note ?? null,
          noteText: note?.short ?? null,
          noteY: note ? top + PATH_HEIGHT + 14 : null,
          dormant: isSleeping,
          x: COLUMNS.path.x,
          y: top,
          width: COLUMNS.path.width,
          height: PATH_HEIGHT,
          centerY: boxCenter,
        });
        edges.push({
          key: `path:${key}`,
          kind: 'path',
          d: `M${TASK_RIGHT} ${centerY} H${PATH_BUS_X} V${boxCenter} H${COLUMNS.path.x}`,
          dormant: isSleeping,
          emphasis: selected,
        });
        top += heights[pathIndex] + PATH_GAP;
      });
      centers.push(centerY);
      y += content + ROW_GAP;
    }
    y -= ROW_GAP;
    const centerY = (centers[0] + centers[centers.length - 1]) / 2;
    const count = group.items.length;
    executors.push({
      key: group.key,
      label: group.label,
      name: fitText(group.label, EXECUTOR_TEXT, 14),
      detail: sleeping
        ? `休眠 · ${count} 条`
        : group.rootId === null
          ? `未认领 · ${count} 条`
          : `${count} 条任务`,
      dormant: sleeping,
      x: COLUMNS.executor.x,
      y: centerY - EXECUTOR_HEIGHT / 2,
      width: COLUMNS.executor.width,
      height: EXECUTOR_HEIGHT,
      centerY,
    });
    for (const task of tasks.filter((row) => row.executorKey === group.key))
      edges.push({
        key: `claim:${task.id}`,
        kind: 'claim',
        d: `M${EXECUTOR_RIGHT} ${centerY} H${EXECUTOR_BUS_X} V${task.centerY} H${COLUMNS.task.x - 4}`,
        dormant: task.dormant,
        emphasis: task.selected,
      });
  });
  const height = groups.length ? y + BOTTOM : TOP + BOTTOM;

  return {
    width: TOPOLOGY_WIDTH,
    height,
    executors,
    tasks,
    paths,
    edges,
    brackets: brackets(paths, overlaps),
    markers: placeMarkers(markers, tasks, paths),
    overlaps,
    dormant: [...dormant].sort(byText),
    omitted,
    hidden,
    unfinished: unfinished.length,
  };
}

function samePath(left: string, right: string): boolean {
  return left === right || left.toLowerCase() === right.toLowerCase();
}

/** 两端都画出来的重叠才连括号；括号竖线彼此交叠时逐级右移，免得叠成一根。 */
function brackets(
  paths: readonly PathRow[],
  overlaps: readonly PathOverlap[],
): OverlapBracket[] {
  const rows = new Map(paths.map((row) => [row.key, row]));
  const spans = overlaps
    .flatMap((overlap) => {
      const a = rows.get(pathKey(overlap.a.taskId, overlap.a.path));
      const b = rows.get(pathKey(overlap.b.taskId, overlap.b.path));
      if (!a || !b) return [];
      return [
        {
          key: `${a.key}\u0000${b.key}`,
          top: Math.min(a.centerY, b.centerY),
          bottom: Math.max(a.centerY, b.centerY),
        },
      ];
    })
    .sort((left, right) => left.top - right.top || left.bottom - right.bottom);
  const levels: number[] = [];
  return spans.map((span) => {
    let level = levels.findIndex((bottom) => bottom < span.top);
    if (level === -1) {
      level = levels.length;
      levels.push(span.bottom);
    } else levels[level] = span.bottom;
    const x = Math.min(
      BRACKET_LIMIT,
      BRACKET_X + BRACKET_DEPTH + level * BRACKET_STEP,
    );
    return {
      key: span.key,
      d: `M${BRACKET_X} ${span.top} H${x} V${span.bottom} H${BRACKET_X}`,
    };
  });
}

/** 圈号：任务的标在框右上角外侧，路径的标在框右侧；同一目标多个圈号依次错开。定位不到的不画。 */
function placeMarkers(
  markers: readonly LayoutMarker[],
  tasks: readonly TaskRow[],
  paths: readonly PathRow[],
): MarkerPlacement[] {
  const placed: MarkerPlacement[] = [];
  const count = new Map<string, number>();
  const next = (key: string) => {
    const index = count.get(key) ?? 0;
    count.set(key, index + 1);
    return index;
  };
  markers.forEach((marker, index) => {
    const target = marker.target;
    if ('taskId' in target) {
      const row = tasks.find((task) => task.id === target.taskId);
      if (!row) return;
      const cx = TASK_RIGHT + 26 + next(row.id) * 25;
      const cy = row.y - 8;
      placed.push({
        key: `marker:${index}:${row.id}`,
        n: marker.n,
        cx,
        cy,
        r: MARKER_RADIUS,
        leader: { x1: cx - 10, y1: cy + 6, x2: TASK_RIGHT + 2, y2: row.y + 6 },
        label: `待办 ${marker.n}：任务「${row.item.title}」`,
        target: { taskId: row.id },
      });
      return;
    }
    const row =
      paths.find((path) => path.path === target.path) ??
      paths.find((path) => samePath(path.path, target.path));
    if (!row) return;
    const cx = PATH_RIGHT + 26;
    const cy = row.y + 6 + next(row.key) * 24;
    placed.push({
      key: `marker:${index}:${row.key}`,
      n: marker.n,
      cx,
      cy,
      r: MARKER_RADIUS,
      leader: { x1: cx - 11, y1: cy + 4, x2: PATH_RIGHT + 1, y2: cy + 8 },
      label: `待办 ${marker.n}：路径 ${row.path}`,
      target: { pathKey: row.key },
    });
  });
  return placed;
}
