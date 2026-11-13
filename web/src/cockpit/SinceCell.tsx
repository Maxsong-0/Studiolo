import { useId } from 'react';
import type { Since } from './since';
import '../styles/cockpit.css';

const counts = new Intl.NumberFormat('en-US');

/**
 * 左下：自上次查看以来（T-195；设计规范第 6 节）。四个数各带一句说明；
 * 拿不到的显示「—」，原因在说明那一行和悬停提示里；实际可能更多的数字后面带「+」。
 */
export function SinceCell({ since }: { since: Since }) {
  const titleId = useId();
  return (
    <section className="cockpit-since" aria-labelledby={titleId}>
      <div className="cockpit-head">
        <h2 id={titleId}>自上次查看以来</h2>
        <span className="cockpit-head__meta">{since.caption}</span>
      </div>
      <ul className="cockpit-since__stats">
        {since.stats.map((stat) => (
          <li key={stat.key} title={stat.reason}>
            <span className="cockpit-since__value">
              {stat.value === null
                ? '—'
                : `${counts.format(stat.value)}${stat.atLeast ? '+' : ''}`}
            </span>
            <span className="cockpit-since__unit">{stat.unit}</span>
            <span className="cockpit-since__note">
              {stat.value === null ? stat.reason : stat.note}
              {stat.value !== null && stat.atLeast ? '，实际可能更多' : ''}
            </span>
          </li>
        ))}
      </ul>
    </section>
  );
}
