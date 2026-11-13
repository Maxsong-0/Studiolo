import { useId, useState, type ReactNode } from 'react';
import type { OpenSource } from '../types';
import { Button } from '../ui';
import { cssVars } from '../ui/dom';
import type { OverviewState, PolicyPendingState } from './model';
import { entryKey, type PendingEntry, type PendingPlan } from './pending';
import { queueText } from './queueText';
import { WAIT_MARKS, waitFraction, waitLabel } from './wait';
import '../styles/cockpit.css';

/**
 * 「等了多久」的图例：一条刻度，三道线是 10 分、1 时、1 天。
 * 10 分那道只画线不写字：它离 1 时只有两成宽，三个字写上去会和「1时」叠在一起。
 */
function WaitLegend() {
  return (
    <div className="cockpit-queue__legend">
      <span>等了多久</span>
      <span className="cockpit-scale" aria-hidden="true">
        {WAIT_MARKS.map((mark, index) => (
          <span
            key={mark.label}
            className="cockpit-scale__mark"
            style={cssVars({ '--cockpit-at': `${mark.at * 100}%` })}
          >
            {index > 0 ? (
              <span className="cockpit-scale__label">{mark.label}</span>
            ) : null}
          </span>
        ))}
      </span>
      <span className="sr-only">
        刻度条按时长的对数画：10 分在三成处，1 小时在一半处，1 天在九成处。
      </span>
    </div>
  );
}

/** 一行的等待时长：一行等宽文字，下面一条对数刻度条；来源不带时间时写「—」，不画条。 */
function Waited({ ms }: { ms: number | null }) {
  if (ms === null)
    return (
      <span className="cockpit-queue__waited" title="这一件的来源不带时间">
        —<span className="sr-only">，不知道等了多久</span>
      </span>
    );
  return (
    <>
      <span className="cockpit-queue__waited">
        <span className="sr-only">已等 </span>
        {waitLabel(ms)}
      </span>
      <span
        className="cockpit-wait"
        aria-hidden="true"
        style={cssVars({
          '--cockpit-wait': `${(waitFraction(ms) * 100).toFixed(1)}%`,
        })}
      >
        <span className="cockpit-wait__fill" />
      </span>
    </>
  );
}

function Row({
  entry,
  selected,
  labels,
  onToggle,
  onOpen,
}: {
  entry: PendingEntry;
  selected: boolean;
  labels: ReadonlyMap<string, string> | undefined;
  onToggle: () => void;
  onOpen: () => void;
}) {
  const detailId = useId();
  const text = queueText(entry, labels);
  return (
    <li
      className="cockpit-queue__row"
      data-source={entry.source}
      data-selected={selected ? 'true' : undefined}
    >
      <div className="cockpit-queue__index">
        <span className="cockpit-queue__n" aria-hidden="true">
          {String(entry.n).padStart(2, '0')}
        </span>
        <span className="cockpit-queue__kind" title={entry.kind}>
          {entry.kind}
        </span>
      </div>
      <div className="cockpit-queue__body">
        <p className="cockpit-queue__title">
          <button
            type="button"
            className="cockpit-queue__select"
            aria-expanded={selected}
            aria-controls={selected ? detailId : undefined}
            onClick={onToggle}
          >
            {text.title}
          </button>
        </p>
        <p className="cockpit-queue__sub">{text.subtitle}</p>
        {selected ? (
          <div id={detailId} className="cockpit-queue__detail">
            <p className="cockpit-queue__facts">
              <span className="cockpit-queue__facts-label">
                {text.facts.label}
              </span>
              {text.facts.items.map((fact, index) => (
                <span key={index}>{fact}</span>
              ))}
            </p>
            <div className="cockpit-queue__actions">
              <Button
                variant="primary"
                aria-label={text.action.name}
                onClick={onOpen}
              >
                {text.action.primary}
              </Button>
            </div>
          </div>
        ) : null}
      </div>
      <div className="cockpit-queue__wait">
        <Waited ms={entry.waitMs} />
      </div>
      <div className="cockpit-queue__open">
        {selected ? null : (
          <button
            type="button"
            className="cockpit-queue__link"
            aria-label={text.action.name}
            onClick={onOpen}
          >
            {text.action.link} <span aria-hidden="true">→</span>
          </button>
        )}
      </div>
    </li>
  );
}

/** 标题下面的一句说明；`key` 是它说的那一路。 */
interface SourceNote {
  key: 'board' | 'policy' | 'truncated';
  body: ReactNode;
}

/**
 * 哪一路还没到手、少了什么：一路一句，放在标题下面。策略读失败时就地给重试。
 * 每句按来源取键，不按位置：前一句消失（那一路读完了）时，后一句还是原来那个节点，
 * 读屏不会把没变的话再念一遍；同一路从「读取中」变成「失败」是同一个节点换字，会念。
 */
function sourceNotes(
  board: OverviewState,
  policy: PolicyPendingState,
  truncated: boolean,
  onRetryPolicy: () => void,
): SourceNote[] {
  const notes: SourceNote[] = [];
  if (board.kind === 'loading')
    notes.push({ key: 'board', body: '正在读取黑板里等你的任务…' });
  if (board.kind === 'error')
    notes.push({
      key: 'board',
      body: '黑板汇总读取失败，等你的黑板任务没列出来；可在右边重试。',
    });
  if (policy.kind === 'loading')
    notes.push({ key: 'policy', body: '正在读取策略里等你的提案和草案…' });
  if (policy.kind === 'error')
    notes.push({
      key: 'policy',
      body: (
        <>
          策略草案读取失败，等你的提案和草案没列出来：{policy.message}
          <Button
            variant="text"
            size="sm"
            className="cockpit-link"
            onClick={onRetryPolicy}
          >
            重试策略草案
          </Button>
        </>
      ),
    });
  if (truncated)
    notes.push({
      key: 'truncated',
      body: '黑板汇总已截断，可能还有等你的任务没列出来。',
    });
  return notes;
}

/**
 * 左上：等你处理的事（T-195；设计规范第 6 节）。
 * 巨号和「件事等你处理」同一行，下面是一列队列：序号与类型、标题与出处、等了多久、去处理。
 * 选中的那一行多一行事实和一个实心主按钮（全页唯一的黑色实心块），其余行行尾是文字链接；
 * 缺省选中第一行，点哪一行的标题就选中哪一行，再点一次收起。
 * 三路里任何一路没读到，巨号显示「—」，并在标题下说明少了哪一路；已经读到的照常列出。
 */
export function QueueCell({
  plan,
  board,
  policy,
  labels,
  asOf,
  onOpen,
  onOpenTask,
  onOpenPolicy,
  onRetryPolicy,
}: {
  plan: PendingPlan;
  board: OverviewState;
  policy: PolicyPendingState;
  /** 整份黑板汇总的执行者显示名；黑板没读到时不给。 */
  labels: ReadonlyMap<string, string> | undefined;
  /** 时长是截至哪一刻算的（本地钟点）；没有带时间的来源时为 null。 */
  asOf: string | null;
  onOpen: OpenSource;
  onOpenTask: (taskId: string) => void;
  onOpenPolicy: () => void;
  onRetryPolicy: () => void;
}) {
  // undefined＝还没点过，选第一行；null＝点过又收起了。
  const [picked, setPicked] = useState<string | null | undefined>(undefined);
  const { entries, total } = plan;
  const keys = entries.map(entryKey);
  // 点过的那一行在重读之后不在队列里了（已经处理掉）：退回第一行。
  const selected =
    picked === null
      ? null
      : picked !== undefined && keys.includes(picked)
        ? picked
        : (keys[0] ?? null);
  const notes = sourceNotes(board, policy, plan.truncated, onRetryPolicy);
  const settled =
    board.kind !== 'loading' &&
    board.kind !== 'error' &&
    policy.kind !== 'loading' &&
    policy.kind !== 'error';
  const timed = entries.some((entry) => entry.waitMs !== null);

  function open(entry: PendingEntry) {
    if (entry.source === 'board') onOpenTask(entry.item.id);
    else if (entry.source === 'policy') onOpenPolicy();
    else onOpen(entry.action.source);
  }

  return (
    <section className="cockpit-queue" aria-label="等你处理">
      <header
        className="cockpit-queue__head"
        style={cssVars({
          // 「—」比一个数字宽，按两位数留位。
          '--cockpit-digits': String(total === null ? 2 : String(total).length),
        })}
      >
        <div
          className="cockpit-queue__count"
          data-state={
            total === null ? 'unknown' : total === 0 ? 'clear' : 'waiting'
          }
          aria-hidden="true"
        >
          <span className="cockpit-queue__count-value">{total ?? '—'}</span>
        </div>
        <div className="cockpit-queue__heading">
          <h2>
            <span className="sr-only">
              {total === null ? '不知道有几' : `${total} `}
            </span>
            件事等你处理
          </h2>
          {entries.length > 0 ? (
            <p className="cockpit-queue__meta">
              {plan.kinds.map(({ kind, count }) => (
                <span key={kind}>
                  {kind} {count}
                </span>
              ))}
              {plan.oldestWaitMs !== null ? (
                <span className="cockpit-queue__oldest">
                  最久的已等 {waitLabel(plan.oldestWaitMs)}
                </span>
              ) : null}
            </p>
          ) : null}
          {notes.map((note) => (
            <p key={note.key} className="cockpit-note" role="status">
              {note.body}
            </p>
          ))}
        </div>
        {timed ? <WaitLegend /> : null}
      </header>
      {entries.length > 0 ? (
        <ol className="cockpit-queue__list">
          {entries.map((entry, index) => (
            <Row
              key={keys[index]}
              entry={entry}
              selected={keys[index] === selected}
              labels={labels}
              onToggle={() =>
                setPicked(keys[index] === selected ? null : keys[index])
              }
              onOpen={() => open(entry)}
            />
          ))}
        </ol>
      ) : settled ? (
        <p className="cockpit-queue__clear" role="status">
          现在没有事等你。
        </p>
      ) : null}
      {entries.length > 0 ? (
        <p className="cockpit-queue__foot">
          <span>队列到底了</span>
          {asOf && timed ? <span>等了多久算到 {asOf}</span> : null}
        </p>
      ) : null}
    </section>
  );
}
