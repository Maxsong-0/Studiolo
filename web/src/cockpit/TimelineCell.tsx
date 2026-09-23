import { memo, useId, type ReactNode } from 'react';
import { Cell, CellHeader, EmptyState, Lanes, Skeleton } from '../ui';
import type { OverviewState } from './model';
import { overviewRange, recentActivity, timelineLanes } from './timeline';
import '../styles/cockpit.css';

function Legend() {
  return (
    <span className="cockpit-legend">
      <span>
        <span className="cockpit-legend__mark" aria-hidden="true" />
        AI 工作
      </span>
      <span>
        <span
          className="cockpit-legend__mark cockpit-legend__mark--owner"
          aria-hidden="true"
        />
        你的审批
      </span>
    </span>
  );
}

function TimelineBody({ state }: { state: OverviewState }): ReactNode {
  switch (state.kind) {
    case 'loading':
      return <Skeleton lines={3} label="正在读取黑板事件…" />;
    case 'error':
      // 失败的原文与重试在协作拓扑格里，这里只说清楚为什么空着，不重复告警。
      return (
        <p className="cockpit-note">
          黑板汇总读取失败，时间线暂时画不出来；可在协作拓扑格重试。
        </p>
      );
    case 'disabled':
      return (
        <EmptyState title="黑板未启用">
          <p>时间线由黑板事件推出；此工作台尚未启用协作黑板。</p>
        </EmptyState>
      );
    case 'ready': {
      const { overview } = state;
      const range = overviewRange(overview);
      const { lanes, hidden } = timelineLanes(overview, range);
      const feed = recentActivity(overview, range);
      return (
        <>
          <Lanes
            from={range.from}
            to={range.to}
            lanes={lanes}
            ticks={range.ticks}
          />
          {hidden > 0 || overview.eventsTruncated ? (
            <p className="cockpit-note">
              {[
                hidden > 0 ? `另 ${hidden} 位执行者这段时间有活动，未画出` : '',
                overview.eventsTruncated ? '事件已截断，只含最新 1000 条' : '',
              ]
                .filter(Boolean)
                .join('；')}
            </p>
          ) : null}
          {feed.length > 0 ? (
            <ol className="cockpit-feed" aria-label="最近动态">
              {feed.map((entry) => (
                <li key={entry.key} title={entry.raw}>
                  <span className="cockpit-feed__time">{entry.time}</span>
                  <span className="cockpit-feed__who">{entry.who}</span>
                  <span className="cockpit-feed__text">{entry.text}</span>
                </li>
              ))}
            </ol>
          ) : (
            <p className="cockpit-note cockpit-feed--empty">
              这段时间还没有黑板动态。
            </p>
          )}
        </>
      );
    }
  }
}

/**
 * 右中：今天的时间线。按执行者分泳道（黑块 AI 工作、红块你的审批），下接最近三条动态。
 * 只随黑板汇总变化重画：悬停待我动作时驾驶舱会重渲染，泳道不必跟着从头推一遍。
 */
export const TimelineCell = memo(function TimelineCell({
  state,
}: {
  state: OverviewState;
}) {
  const titleId = useId();
  return (
    <Cell
      as="section"
      span={6}
      className="cockpit-timeline"
      aria-labelledby={titleId}
    >
      <CellHeader
        title={<span id={titleId}>今天的时间线</span>}
        meta={state.kind === 'ready' ? <Legend /> : null}
      />
      <TimelineBody state={state} />
    </Cell>
  );
});
