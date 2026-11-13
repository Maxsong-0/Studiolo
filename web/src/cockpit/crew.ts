/**
 * 「谁在干活」（T-195；设计规范第 6 节）：最近 24 小时，每个执行者一条泳道（纯函数）。
 *
 * - 黑段＝它在干活：与协作页「今天的时间线」同一个 todayLanes，只是范围换成最近 24 小时、不设条数上限。
 * - 红段＝停下等你：它名下等你的任务，从最后一次进展一直停到汇总生成那一刻。
 * - 一行摆一条任务：有等你的摆等得最久的那条，否则摆尝试仍在进行、最近有进展的，再否则摆别的未结束任务；
 *   名下没有未结束任务、只是这 24 小时里干过活的，摆它最近结束的那条。
 * - 排序：停下等你的在最前（等得最久的先），其次在跑的（最近有进展的先），最后是别的；最多列
 *   {@link CREW_ROWS} 行，多出的报条数，不静默截断。
 *
 * 全部由黑板汇总推出，不补数据；「现在」一律取汇总生成时刻，不掺本机时钟。
 */
import type { BoardOverview, BoardOverviewItem } from '../boardOverviewModel';
import {
  agentName,
  executorOf,
  overviewLabels,
} from '../collab/executorGrouping';
import {
  OWNER_KEY,
  clockLabel,
  timelineRange,
  todayLanes,
} from '../collab/todayLanes';
import { isDormant, taskStatus } from '../collab/topologyLayout';
import type { LaneBlock } from '../ui';
import { DAY_MS, isFinished, overviewNow, waitingKind } from './model';
import { agoLabel, waitLabel } from './wait';

/** 泳道画多久：最近 24 小时，终点对齐到下一个整点（24 格每格正好一个钟点）。 */
export const CREW_SPAN_MS = DAY_MS;
/** 最多列几个执行者。 */
export const CREW_ROWS = 6;
/** 刻度分四段，每段 6 小时。 */
const TICK_SEGMENTS = 4;

export type CrewState = 'waiting' | 'running' | 'idle';

export interface CrewRow {
  /** 执行者分组键（executorGrouping 的 `instance:…`／`adapter:…`）。 */
  key: string;
  name: string;
  state: CrewState;
  /** 这一行摆出来的那条任务；名下一条任务都没有时为 null。 */
  task: { id: string; title: string } | null;
  /** 除摆出来的这条外，它名下还有几条未结束的任务。 */
  more: number;
  /** 右侧的状态说法：「等你 31 分」「进行中 · 8 分前」「休眠 · 2 天前」。 */
  status: string;
  /** 红段（owner＝停下等你）在前、黑段（work）在后，按这个顺序画；时间戳与 {@link Crew} 的 from／to 同一单位。 */
  blocks: LaneBlock[];
  /** 给读屏的一句话：这条泳道画了什么。 */
  label: string;
}

export interface Crew {
  from: number;
  to: number;
  /** 汇总生成时刻。 */
  now: number;
  /** 刻度：位置（0 到 1）与本地时刻；跨到前一天的第一格带「昨天」。 */
  ticks: { at: number; label: string }[];
  rows: CrewRow[];
  /** 该列而没列出来的执行者数（超过 {@link CREW_ROWS}）。 */
  hidden: number;
  /** 全部执行者里在跑的、停下等你的各几个（不受行数上限影响）。 */
  running: number;
  waiting: number;
}

const lastProgress = (item: BoardOverviewItem): number | null => {
  const last = item.progress?.lastProgressMs ?? null;
  return last !== null && last > 0 ? last : null;
};

/** 越早越靠前，没有时间的排最后。 */
const oldestFirst = (left: number | null, right: number | null) =>
  left === null ? (right === null ? 0 : 1) : right === null ? -1 : left - right;
/** 越近越靠前，没有时间的排最后。 */
const newestFirst = (left: number | null, right: number | null) =>
  left === null ? (right === null ? 0 : 1) : right === null ? -1 : right - left;

const byText = (left: string, right: string) =>
  left < right ? -1 : left > right ? 1 : 0;

function ticks(from: number, to: number, now: number) {
  const today = new Date(now).toDateString();
  return Array.from({ length: TICK_SEGMENTS }, (_, index) => {
    const ms = from + ((to - from) * index) / TICK_SEGMENTS;
    const earlier = index === 0 && new Date(ms).toDateString() !== today;
    return {
      at: index / TICK_SEGMENTS,
      label: `${earlier ? '昨天 ' : ''}${clockLabel(ms)}`,
    };
  });
}

interface Group {
  key: string;
  name: string;
  items: BoardOverviewItem[];
  work: LaneBlock[];
}

function toRow(
  group: Group,
  now: number,
): { row: CrewRow; order: number | null } {
  const unfinished = group.items.filter((item) => !isFinished(item));
  const waiting = unfinished
    .filter((item) => waitingKind(item) !== null)
    .sort((a, b) => oldestFirst(lastProgress(a), lastProgress(b)));
  const moving = unfinished
    .filter(
      (item) =>
        waitingKind(item) === null &&
        item.attempt?.active === true &&
        !isDormant(item, now),
    )
    .sort((a, b) => newestFirst(lastProgress(a), lastProgress(b)));
  const state: CrewState =
    waiting.length > 0 ? 'waiting' : moving.length > 0 ? 'running' : 'idle';
  const shown =
    waiting[0] ??
    moving[0] ??
    [...unfinished].sort((a, b) =>
      newestFirst(lastProgress(a), lastProgress(b)),
    )[0] ??
    [...group.items].sort((a, b) =>
      newestFirst(lastProgress(a), lastProgress(b)),
    )[0];
  const progressed = shown ? lastProgress(shown) : null;
  const ago = progressed === null ? null : Math.max(0, now - progressed);

  let status = '';
  if (state === 'waiting')
    status = ago === null ? '等你' : `等你 ${waitLabel(ago)}`;
  else if (shown) {
    const said = taskStatus(shown, now).status;
    status = ago === null ? said : `${said} · ${agoLabel(ago)}`;
  }

  const stopped: LaneBlock[] = waiting.flatMap((item) => {
    const since = lastProgress(item);
    return since === null || since > now
      ? []
      : [{ start: since, end: now, tone: 'owner' as const }];
  });
  const lastWork = group.work.reduce<number | null>(
    (latest, block) =>
      latest === null ? block.end : Math.max(latest, block.end),
    null,
  );
  return {
    row: {
      key: group.key,
      name: group.name,
      state,
      task: shown ? { id: shown.id, title: shown.title } : null,
      more: unfinished.length - (shown && !isFinished(shown) ? 1 : 0),
      status,
      // 红段在下、黑段在上：等你的这段时间里它又干了别的活，黑段照样看得见。
      blocks: [...stopped, ...group.work],
      label: `${group.name}，最近 24 小时：${
        group.work.length > 0
          ? `干活 ${group.work.length} 段`
          : '没有干活的记录'
      }${state === 'waiting' ? '，现在停下等你' : ''}`,
    },
    // 同一档里的先后：等你的看最早停下的，在跑的看最近的进展，别的看最近干活到什么时候。
    order:
      state === 'waiting'
        ? (waiting.map(lastProgress).find((since) => since !== null) ?? null)
        : state === 'running'
          ? progressed
          : lastWork,
  };
}

const rank: Record<CrewState, number> = { waiting: 0, running: 1, idle: 2 };

/**
 * 从一份黑板汇总推出「谁在干活」。一个执行者列出来的条件：这 24 小时里干过活，
 * 或者名下还有未结束的任务。从未被认领的任务没有执行者，不在这里（协作页看得到）。
 */
export function crew(overview: BoardOverview): Crew {
  const now = overviewNow(overview);
  const to = timelineRange(now).to;
  const from = to - CREW_SPAN_MS;
  const labels = overviewLabels(overview);
  const lanes = todayLanes(overview, {
    range: { from, to, ticks: [] },
    labels,
    maxLanes: Infinity,
  }).lanes.filter((lane) => lane.key !== OWNER_KEY);

  const groups = new Map<string, Group>();
  const groupOf = (key: string, name: string) => {
    let group = groups.get(key);
    if (!group) {
      group = { key, name, items: [], work: [] };
      groups.set(key, group);
    }
    return group;
  };
  for (const lane of lanes)
    groupOf(lane.key, lane.name).work.push(...lane.blocks);
  const held = overview.items.filter((item) => item.holder !== null);
  for (const item of held) {
    if (isFinished(item)) continue;
    const identity = executorOf(item);
    groupOf(
      identity.key,
      labels.get(identity.key) ?? agentName(identity.agentType),
    ).items.push(item);
  }
  // 已结束的任务只跟着已经上榜的执行者走：一条昨天收尾、今天没动静的任务不单独撑起一行。
  for (const item of held)
    if (isFinished(item)) groups.get(executorOf(item).key)?.items.push(item);

  const all = [...groups.values()]
    .map((group) => toRow(group, now))
    .sort(
      (left, right) =>
        rank[left.row.state] - rank[right.row.state] ||
        (left.row.state === 'waiting'
          ? oldestFirst(left.order, right.order)
          : newestFirst(left.order, right.order)) ||
        byText(left.row.name, right.row.name) ||
        byText(left.row.key, right.row.key),
    )
    .map(({ row }) => row);
  return {
    from,
    to,
    now,
    ticks: ticks(from, to, now),
    rows: all.slice(0, CREW_ROWS),
    hidden: Math.max(0, all.length - CREW_ROWS),
    running: all.filter((row) => row.state === 'running').length,
    waiting: all.filter((row) => row.state === 'waiting').length,
  };
}
