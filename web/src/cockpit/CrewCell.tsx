import { memo, useId, useMemo, type ReactNode } from 'react';
import { clockLabel, recentActivity } from '../collab/todayLanes';
import { Button, EmptyState, ErrorState, Skeleton } from '../ui';
import { laneBoxes } from '../ui/geometry';
import { crew, type Crew, type CrewRow } from './crew';
import type { OverviewState } from './model';
import '../styles/cockpit.css';

/** 「最近动态」列几条。 */
const FEED = 5;

/** 一条 24 小时泳道：黑段在干活，红段停下等你。整条是一幅图，说法进 aria-label。 */
function Lane({ row, from, to }: { row: CrewRow; from: number; to: number }) {
  return (
    <span className="cockpit-crew__lane" role="img" aria-label={row.label}>
      <svg
        className="cockpit-crew__svg"
        viewBox="0 0 100 10"
        preserveAspectRatio="none"
      >
        {laneBoxes(from, to, row.blocks).map((box, index) => (
          <rect
            key={index}
            className={`ui-lanes__block ui-lanes__block--${box.tone}`}
            x={box.left}
            width={box.width}
            y={0}
            height={10}
          />
        ))}
      </svg>
    </span>
  );
}

function CrewList({ data }: { data: Crew }) {
  return (
    <>
      <div className="cockpit-crew__scale" aria-hidden="true">
        {data.ticks.map((tick) => (
          <span key={tick.at}>{tick.label}</span>
        ))}
      </div>
      <ul className="cockpit-crew__list">
        {data.rows.map((row) => (
          <li key={row.key} data-state={row.state}>
            <div className="cockpit-crew__line">
              <span className="cockpit-crew__name" title={row.name}>
                {row.name}
              </span>
              {/* 标题放不下时只截标题：「另 N 条」是一个数，不能跟着被截掉 */}
              <span className="cockpit-crew__what">
                <span className="cockpit-crew__task" title={row.task?.title}>
                  {row.task ? row.task.title : '名下没有任务'}
                </span>
                {row.more > 0 ? (
                  <span className="cockpit-crew__more">另 {row.more} 条</span>
                ) : null}
              </span>
              <span className="cockpit-crew__status">{row.status}</span>
            </div>
            <Lane row={row} from={data.from} to={data.to} />
          </li>
        ))}
      </ul>
      {data.hidden > 0 ? (
        <p className="cockpit-note">
          另 {data.hidden} 位执行者没列出，协作页看得到。
        </p>
      ) : null}
    </>
  );
}

function CrewBody({
  state,
  data,
  onRetry,
}: {
  state: OverviewState;
  data: Crew | null;
  onRetry: () => void;
}): ReactNode {
  switch (state.kind) {
    case 'loading':
      return <Skeleton lines={4} label="正在读取黑板汇总…" />;
    case 'error':
      return (
        <ErrorState
          title="黑板汇总读取失败"
          onRetry={onRetry}
          retryLabel="重试黑板汇总"
        >
          <p>{state.message}</p>
        </ErrorState>
      );
    case 'disabled':
      return (
        <EmptyState title="黑板未启用">
          <p>
            此工作台尚未启用协作黑板（汇总接口返回
            404），谁在干活、最近动态和黑板读数暂不可用；其余照常。
          </p>
        </EmptyState>
      );
    case 'ready':
      return data && data.rows.length > 0 ? (
        <CrewList data={data} />
      ) : (
        <EmptyState title="最近 24 小时没有执行者在干活" />
      );
  }
}

/**
 * 右上：谁在干活（T-195；设计规范第 6 节）。最近 24 小时每个执行者一条泳道，黑段在干活、红段停下等你，
 * 停下等你的排最前；下接最近几条动态。泳道的黑段与协作页「今天的时间线」同一个 todayLanes。
 * 只随黑板汇总变化重画：左边队列换选中行时驾驶舱会重渲染，泳道不必跟着从头推一遍。
 */
export const CrewCell = memo(function CrewCell({
  state,
  onRetry,
  onOpenCollab,
}: {
  state: OverviewState;
  onRetry: () => void;
  onOpenCollab: () => void;
}) {
  const titleId = useId();
  const feedId = useId();
  const overview = state.kind === 'ready' ? state.overview : null;
  const data = useMemo(() => (overview ? crew(overview) : null), [overview]);
  const feed = useMemo(
    () =>
      overview && data
        ? recentActivity(
            overview,
            { from: data.from, to: data.to, ticks: [] },
            FEED,
          )
        : [],
    [overview, data],
  );
  return (
    <aside className="cockpit-crew" aria-labelledby={titleId}>
      <div className="cockpit-head">
        <h2 id={titleId}>谁在干活</h2>
        {data ? (
          <span className="cockpit-head__meta">
            {data.running} 个在跑 · {data.waiting} 个停下等你
          </span>
        ) : null}
      </div>
      <CrewBody state={state} data={data} onRetry={onRetry} />
      {data ? (
        <section className="cockpit-crew__feed" aria-labelledby={feedId}>
          <div className="cockpit-head">
            <h3 id={feedId}>最近动态</h3>
            <span className="cockpit-head__meta">
              截至 {clockLabel(data.now)}
            </span>
          </div>
          {feed.length > 0 ? (
            <ol className="cockpit-feed">
              {feed.map((entry) => (
                <li key={entry.key} title={entry.raw}>
                  <span className="cockpit-feed__time">{entry.time}</span>
                  <span className="cockpit-feed__text">
                    <span className="cockpit-feed__who">{entry.who}</span>{' '}
                    {entry.text}
                  </span>
                </li>
              ))}
            </ol>
          ) : (
            <p className="cockpit-note">最近 24 小时还没有黑板动态。</p>
          )}
          <Button
            variant="text"
            size="sm"
            className="cockpit-link"
            onClick={onOpenCollab}
          >
            去协作页看全部
          </Button>
        </section>
      ) : null}
    </aside>
  );
});
