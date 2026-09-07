import type { Action, OpenSource } from '../types';
import { Icon } from './Icon';

function displayTitle(action: Action) {
  return action.title.startsWith(`${action.kind}：`)
    ? action.title.slice(action.kind.length + 1)
    : action.title;
}

export function Inbox({
  actions,
  onOpen,
  onViewAll,
  heading = true,
}: {
  actions: Action[];
  onOpen: OpenSource;
  onViewAll?: () => void;
  heading?: boolean;
}) {
  return (
    <section className="inbox" aria-label="待我动作">
      {heading ? (
        <div className="section-heading">
          <h2>待我动作</h2>
          {onViewAll ? (
            <button className="text-button" onClick={onViewAll}>
              查看全部
            </button>
          ) : null}
        </div>
      ) : null}
      {actions.length === 0 ? (
        <p className="empty-state" role="status">
          当前没有待我动作。
        </p>
      ) : (
        <ul className="action-list">
          {actions.map((action) => (
            <li className="action-row" key={action.id}>
              <button
                className="action-open"
                onClick={() => onOpen(action.source)}
                aria-label={`${action.title}，查看原文`}
              >
                <span
                  className={`action-kind action-kind-${action.kind === '待重启' ? 'restart' : action.kind === '待拍板' ? 'decision' : 'review'}`}
                >
                  <Icon
                    name={
                      action.kind === '待重启'
                        ? 'refresh'
                        : action.kind === '待拍板'
                          ? 'decision'
                          : 'check'
                    }
                  />
                  {action.kind || '未知'}
                </span>
                <span className="action-title">{displayTitle(action)}</span>
                <span className="action-project">{action.project_name}</span>
                <span className="action-source">
                  {action.source.path} · 第 {action.source.line} 行
                </span>
                <span className="action-link">
                  查看原文 <Icon name="arrow" />
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
