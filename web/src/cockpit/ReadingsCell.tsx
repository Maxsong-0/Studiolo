import { useId } from 'react';
import { LiveStamp } from '../live/LiveStamp';
import { Button } from '../ui';
import { readingsSummary, type Reading } from './readings';
import '../styles/cockpit.css';

/**
 * 右下：系统读数（T-195；设计规范第 6 节）。八格两列，要看的那一格黑底反白；
 * 读不到的显示「—」，原因在悬停提示和读屏文字里。标题下一行是日期与副本代次，后面跟着这一页的数是
 * 多久以前读的（T-196，自己走字）；数据提示的入口也在这一行。
 */
export function ReadingsCell({
  readings,
  snapshot,
  diagnostics,
  onShowDiagnostics,
}: {
  readings: readonly Reading[];
  /** 日期与副本代次那一行（readings.ts 的 snapshotLine）。 */
  snapshot: string;
  /** 数据提示条数；条目本身在顶栏状态浮层里。 */
  diagnostics: number;
  onShowDiagnostics: () => void;
}) {
  const titleId = useId();
  return (
    <section className="cockpit-readings" aria-labelledby={titleId}>
      <div className="cockpit-head">
        <h2 id={titleId}>系统读数</h2>
        <span className="cockpit-head__meta">{readingsSummary(readings)}</span>
      </div>
      <p className="cockpit-readings__snapshot">
        <span className="cockpit-readings__when">
          <span>{snapshot}</span>
          <LiveStamp />
        </span>
        {diagnostics > 0 ? (
          <Button
            variant="text"
            size="sm"
            className="cockpit-link"
            aria-label={`${diagnostics} 条数据提示，查看数据提示`}
            onClick={onShowDiagnostics}
          >
            {diagnostics} 条数据提示
          </Button>
        ) : null}
      </p>
      <ul className="cockpit-readings__cells">
        {readings.map((reading) => (
          <li
            key={reading.label}
            className={
              reading.alert
                ? 'cockpit-readings__cell ui-on-ink'
                : 'cockpit-readings__cell'
            }
            data-alert={reading.alert ? 'true' : undefined}
            title={reading.reason}
          >
            <span className="cockpit-readings__label">{reading.label}</span>
            <span className="cockpit-readings__value">
              {reading.value ?? '—'}
              {reading.alert ? <span className="sr-only">，要看</span> : null}
            </span>
            {reading.reason ? (
              <span className="sr-only">{reading.reason}</span>
            ) : null}
          </li>
        ))}
      </ul>
    </section>
  );
}
