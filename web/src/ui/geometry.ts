/* 图形组件的纯几何：只做换算与裁剪，不补数据，不合法的输入直接不画。 */

export type LaneTone = 'work' | 'owner';

export interface LaneBlock {
  start: number;
  end: number;
  tone: LaneTone;
}

/** 泳道块在轨道上的位置，单位是轨道宽度的百分比。 */
export interface LaneBox {
  left: number;
  width: number;
  tone: LaneTone;
}

/** 最窄的块占轨道宽度的百分比：一次瞬时审批也要看得见。 */
export const LANE_MIN_WIDTH = 0.6;

const round = (value: number) => Math.round(value * 1000) / 1000;

/**
 * 把 [start, end] 裁剪到 [from, to] 后换成百分比。
 * 与范围不相交的块、首尾颠倒或非有限数的块不画；过窄的块按最小宽度画，
 * 并向左让位，保证不越过轨道右缘。
 */
export function laneBoxes(
  from: number,
  to: number,
  blocks: readonly LaneBlock[],
): LaneBox[] {
  const span = to - from;
  if (!Number.isFinite(span) || span <= 0) return [];
  const boxes: LaneBox[] = [];
  for (const block of blocks) {
    const { start, end, tone } = block;
    if (!Number.isFinite(start) || !Number.isFinite(end) || end < start) {
      continue;
    }
    const instant = start === end;
    const outside = instant
      ? start < from || start > to
      : end <= from || start >= to;
    if (outside) continue;
    const clippedStart = Math.max(start, from);
    const clippedEnd = Math.min(end, to);
    const width = Math.min(
      100,
      Math.max(LANE_MIN_WIDTH, ((clippedEnd - clippedStart) / span) * 100),
    );
    const left = Math.min(
      Math.max(0, ((clippedStart - from) / span) * 100),
      100 - width,
    );
    boxes.push({ left: round(left), width: round(width), tone });
  }
  return boxes;
}

/** 柱顶留出的空隙，免得贴顶的限值虚线被裁掉一半。 */
export const SPARK_HEADROOM = 2;

export interface SparkbarsGeometry {
  /** viewBox 宽度：每个值占 1 个单位。 */
  width: number;
  /** 每根柱的中心 x 与柱顶 y；柱底都在 height。 */
  bars: { x: number; top: number }[];
  /** 限值虚线的 y；没有限值时为 null。 */
  limitY: number | null;
}

/**
 * 柱高按 max（缺省取数据与限值中的最大者）线性换算；负数与非有限数按 0 画，超过 max 的柱顶到顶。
 */
export function sparkbarsGeometry(
  values: readonly number[],
  height: number,
  limit?: number | null,
  max?: number | null,
): SparkbarsGeometry {
  const clean = values.map((value) =>
    Number.isFinite(value) && value > 0 ? value : 0,
  );
  const limitValue =
    limit != null && Number.isFinite(limit) && limit >= 0 ? limit : null;
  const top =
    max != null && Number.isFinite(max) && max > 0
      ? max
      : Math.max(0, limitValue ?? 0, ...clean);
  const usable = Math.max(0, height - SPARK_HEADROOM);
  const y = (value: number) =>
    top > 0 ? height - (Math.min(value, top) / top) * usable : height;
  return {
    width: Math.max(1, clean.length),
    bars: clean.map((value, index) => ({
      x: index + 0.5,
      top: round(y(value)),
    })),
    limitY: limitValue === null ? null : round(y(limitValue)),
  };
}
