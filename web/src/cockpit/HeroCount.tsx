import type { ReactNode } from 'react';
import { BigNumber, Cell } from '../ui';
import { DAY_MS, type OverviewState } from './model';
import '../styles/cockpit.css';

/** 「最久 N 天」：满一天按天数，不满一天如实说「最久不到 1 天」。 */
function Waited({ ms }: { ms: number }) {
  const days = Math.floor(ms / DAY_MS);
  return days >= 1 ? (
    <span>
      最久 <span className="cockpit-signal">{days} 天</span>
    </span>
  ) : (
    <span>
      最久<span className="cockpit-signal">不到 1 天</span>
    </span>
  );
}

/**
 * 左上巨号：待我动作总数（两位数补零），下接「件事等你处理」与等宽小字。
 * 「最久」只算自带时间的黑板条目；黑板还在读或读取失败时总数不可知，显示「—」并说明。
 */
export function HeroCount({
  total,
  oldestWaitMs,
  marked,
  truncated,
  board,
}: {
  total: number | null;
  oldestWaitMs: number | null;
  /** 标在拓扑缩略上的条数。 */
  marked: number;
  truncated: boolean;
  board: OverviewState['kind'];
}) {
  let note: ReactNode = null;
  if (board === 'loading') note = '正在读取黑板，总数稍后给出';
  else if (board === 'error') note = '黑板读取失败，总数暂不可得';
  else {
    const parts: ReactNode[] = [];
    if (oldestWaitMs !== null)
      parts.push(<Waited key="oldest" ms={oldestWaitMs} />);
    if (marked > 0) parts.push(`${marked} 件标在右图`);
    if (truncated) parts.push('黑板汇总已截断，可能还有更多');
    note = parts.length
      ? parts.flatMap((part, index) => (index === 0 ? [part] : ['，', part]))
      : null;
  }
  return (
    <Cell as="div" span={3} className="cockpit-hero">
      <span className="cockpit-kicker">待我动作</span>
      <BigNumber value={total} label="件事等你处理" note={note} />
    </Cell>
  );
}
