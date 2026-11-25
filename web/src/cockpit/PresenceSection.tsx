import { useId, useMemo } from 'react';
import { clockLabel } from '../collab/todayLanes';
import type { Presence } from '../presenceModel';
import { Button } from '../ui';
import { presenceGroup, type PresenceRow } from './presence';
import type { PresenceReadState } from './usePresenceRead';

function Row({ row }: { row: PresenceRow }) {
  return (
    <li data-state={row.state}>
      <div className="cockpit-crew__line">
        <span className="cockpit-crew__vendor">{row.vendor}</span>
        <span className="cockpit-crew__what">
          <span className="cockpit-crew__task" title={row.title}>
            {row.where}
          </span>
          {row.detail ? (
            <span className="cockpit-crew__more">{row.detail}</span>
          ) : null}
        </span>
        <span className="cockpit-crew__status">{row.status}</span>
      </div>
    </li>
  );
}

function Sessions({
  presence,
  titleId,
}: {
  presence: Presence;
  titleId: string;
}) {
  const group = useMemo(() => presenceGroup(presence), [presence]);
  const minutes = Math.round(presence.expiresInSecs / 60);
  return (
    <>
      <div className="cockpit-head">
        <h3 id={titleId}>本机会话</h3>
        {group.rows.length > 0 ? (
          <span className="cockpit-head__meta">
            {group.running} 个在跑 · {group.waiting} 个等你
          </span>
        ) : null}
      </div>
      {group.rows.length > 0 ? (
        <ul className="cockpit-crew__list">
          {group.rows.map((row) => (
            <Row key={row.key} row={row} />
          ))}
        </ul>
      ) : null}
      <p className="cockpit-note">
        {group.hidden > 0 ? `另 ${group.hidden} 条没列出。` : ''}
        {group.rows.length > 0 ? '' : '现在没有推来的会话状态。'}
        {`只含认得准是你自己开的会话，实际可能更多；${minutes} 分钟没有新状态就不显示。从 ${clockLabel(Date.parse(presence.since))} 起攒。`}
      </p>
    </>
  );
}

/**
 * 「谁在干活」里的「本机会话」（T-237，ADR-046）：连接器推来的会话状态，不是黑板任务。
 * 自己一个接口、自己成败：读不到只影响这一组；本机没装黑板就整组不出现。
 */
export function PresenceSection({
  state,
  onRetry,
}: {
  state: PresenceReadState;
  onRetry: () => void;
}) {
  const titleId = useId();
  if (state.kind === 'disabled') return null;
  return (
    <section className="cockpit-crew__sessions" aria-labelledby={titleId}>
      {state.kind === 'ready' ? (
        <Sessions presence={state.presence} titleId={titleId} />
      ) : (
        <>
          <div className="cockpit-head">
            <h3 id={titleId}>本机会话</h3>
          </div>
          {state.kind === 'loading' ? (
            <p className="cockpit-note">正在读取本机会话…</p>
          ) : (
            <>
              <p className="cockpit-note">本机会话读不到：{state.message}</p>
              <Button
                variant="text"
                size="sm"
                className="cockpit-link"
                onClick={onRetry}
              >
                重试本机会话
              </Button>
            </>
          )}
        </>
      )}
    </section>
  );
}
